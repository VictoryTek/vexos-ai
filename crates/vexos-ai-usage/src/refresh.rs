//! One refresh: decide whether to ask, ask, and write the record.
//!
//! The order matters for safety: when usage is switched off, nothing is read at
//! all (not the login file, not the network). Only a switched-on service ever
//! opens Claude's sign-in.

use std::cell::Cell;
use std::path::PathBuf;

use crate::credentials::{self, NoLogin};
use crate::parse::{Window, parse_usage};
use crate::probe::{Fetch, FetchError};
use crate::state::{self, Record, Status, zero_elapsed};

/// A request is never made twice inside this, unless a person forces it.
pub const MIN_PROBE_MS: u64 = 15_000;
/// After nothing answered (no route yet after login), try again soon. This does
/// not count as a failure.
pub const TRANSPORT_RETRY_MS: u64 = 60_000;
/// After the sign-in was refused, asking again soon changes nothing.
pub const REFUSED_RETRY_MS: u64 = 15 * 60_000;
/// The backoff: 1, 2, 4 ... 60 minutes.
pub const MAX_BACKOFF_MS: u64 = 60 * 60_000;
/// Numbers measured this recently still describe the account, even when Anthropic
/// has just said to ask later.
pub const CURRENT_ENOUGH_MS: u64 = 15 * 60_000;

/// Everything a refresh needs from outside, so a test can supply it.
pub struct Env {
    pub now_ms: u64,
    pub home: PathBuf,
    /// `CLAUDE_CONFIG_DIR`, when set.
    pub claude_config_dir: Option<String>,
    pub state_file: PathBuf,
    /// The panel's "show plan limits" switch. Off reads nothing.
    pub enabled: bool,
    pub force: bool,
}

/// 1, 2, 4 ... minutes for the n-th failure in a row (n >= 1), capped at an hour.
pub fn backoff_ms(failures: u32) -> u64 {
    let shift = failures.saturating_sub(1).min(16);
    (60_000u64 << shift).min(MAX_BACKOFF_MS)
}

fn kept(previous: &Record, now: u64) -> Vec<Window> {
    zero_elapsed(&previous.windows, now)
}

/// Runs one refresh, writes the record, and returns it. A write that fails is
/// not an error for the caller: the record is returned all the same.
pub fn refresh(env: &Env, fetcher: &dyn Fetch) -> Record {
    let previous = state::read(&env.state_file).unwrap_or_else(Record::empty);
    let (record, write) = decide(env, fetcher, previous);
    if write {
        let _ = state::write(&env.state_file, &record);
    }
    record
}

fn decide(env: &Env, fetcher: &dyn Fetch, previous: Record) -> (Record, bool) {
    let now = env.now_ms;

    // Switched off: forget the numbers, read nothing.
    if !env.enabled {
        let record = Record {
            updated_at_ms: now,
            ..Record::empty()
        };
        return (record, true);
    }

    let base = Record {
        updated_at_ms: now,
        enabled: true,
        plan: previous.plan.clone(),
        fetched_at_ms: previous.fetched_at_ms,
        probed_at_ms: previous.probed_at_ms,
        next_at_ms: previous.next_at_ms,
        failures: previous.failures,
        ..Record::empty()
    };
    let with_kept = |status: Status| {
        let windows = kept(&previous, now);
        Record {
            status,
            stale: !windows.is_empty(),
            windows,
            ..base.clone()
        }
    };

    let login = match credentials::read_login(&env.home, env.claude_config_dir.as_deref()) {
        Ok(login) => login,
        Err(NoLogin) => return (with_kept(Status::WaitingForAuth), true),
    };

    // A lapsed access token is routine, not a sign-out: it lives about eight
    // hours, and the CLI mints a new one the next time it runs. Only a lapsed
    // refresh token is a sign-in a person must redo.
    if login.expires_at_ms > 0 && login.expires_at_ms <= now {
        let status = if login.refresh_expires_at_ms > now {
            Status::Paused
        } else {
            Status::SignInExpired
        };
        return (
            Record {
                plan: login.plan,
                ..with_kept(status)
            },
            true,
        );
    }

    // A backoff is never overruled, not even by a person pressing refresh. The
    // one exception is the wait after a refused sign-in: a person who signed in
    // again, as the panel told them, must not wait out the rest of it.
    let refused_wait = previous.status == Status::SignInExpired;
    if now < previous.next_at_ms && !(env.force && refused_wait) {
        return (previous, false);
    }
    // The interval absorbs repeated opens; a forced refresh skips it.
    if !env.force
        && previous.status == Status::Ok
        && previous.enabled
        && now.saturating_sub(previous.probed_at_ms) < MIN_PROBE_MS
    {
        return (previous, false);
    }

    let outcome = fetcher
        .fetch(&login.token)
        .map_err(Failure::Fetch)
        .and_then(|body| parse_usage(&body).map_err(|_| Failure::Parse));

    let record = match outcome {
        Ok(windows) => Record {
            status: Status::Ok,
            plan: login.plan,
            windows,
            stale: false,
            fetched_at_ms: now,
            probed_at_ms: now,
            next_at_ms: 0,
            failures: 0,
            ..base
        },
        Err(failure) => {
            let (status, failures, next_at) = failure.outcome(previous.failures, now);
            let mut record = Record {
                plan: login.plan,
                probed_at_ms: now,
                failures,
                next_at_ms: next_at,
                ..with_kept(status)
            };
            // Anthropic rate-limits this endpoint readily, and a 429 only says
            // "ask later": numbers measured minutes ago still describe the account.
            if status == Status::RateLimited
                && !record.windows.is_empty()
                && now.saturating_sub(previous.fetched_at_ms) < CURRENT_ENOUGH_MS
            {
                record.stale = false;
            }
            record
        }
    };
    (record, true)
}

enum Failure {
    Fetch(FetchError),
    Parse,
}

impl Failure {
    /// The status, the failures-in-a-row count, and the earliest next request.
    fn outcome(&self, failures: u32, now: u64) -> (Status, u32, u64) {
        let counted = |status: Status, delay: Option<u64>| {
            let n = failures.saturating_add(1);
            (status, n, now + delay.unwrap_or_else(|| backoff_ms(n)))
        };
        match self {
            Failure::Fetch(FetchError::Status {
                code: 429,
                retry_after_secs,
            }) => counted(
                Status::RateLimited,
                retry_after_secs
                    .map(|s| s.saturating_mul(1000).clamp(MIN_PROBE_MS, MAX_BACKOFF_MS)),
            ),
            Failure::Fetch(FetchError::Status {
                code: 401 | 403, ..
            }) => (Status::SignInExpired, failures, now + REFUSED_RETRY_MS),
            Failure::Fetch(FetchError::Transport) => {
                (Status::Unreachable, failures, now + TRANSPORT_RETRY_MS)
            }
            Failure::Fetch(_) => counted(Status::Unavailable, None),
            Failure::Parse => counted(Status::FormatChanged, None),
        }
    }
}

/// A fetcher for tests and for the "never asked" assertions: counts calls.
pub struct Counting<'a> {
    pub calls: &'a Cell<u32>,
    pub reply: Result<String, FetchError>,
}

impl Fetch for Counting<'_> {
    fn fetch(&self, _token: &crate::secret::Secret) -> Result<String, FetchError> {
        self.calls.set(self.calls.get() + 1);
        match &self.reply {
            Ok(body) => Ok(body.clone()),
            Err(FetchError::Status {
                code,
                retry_after_secs,
            }) => Err(FetchError::Status {
                code: *code,
                retry_after_secs: *retry_after_secs,
            }),
            Err(FetchError::Transport) => Err(FetchError::Transport),
            Err(FetchError::TooLarge) => Err(FetchError::TooLarge),
            Err(FetchError::Redirect) => Err(FetchError::Redirect),
            Err(FetchError::Other) => Err(FetchError::Other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    const NOW: u64 = 1_800_000_000_000;
    const BODY: &str = r#"{"five_hour":{"utilization":12,"resets_at":"2027-01-15T10:00:00Z"},
                          "seven_day":{"utilization":40,"resets_at":"2027-01-20T10:00:00Z"}}"#;

    struct Rig {
        home: PathBuf,
    }

    impl Rig {
        fn new(name: &str, login: Option<&str>) -> Rig {
            let home = std::env::temp_dir()
                .join(format!("vexos-usage-refresh-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&home);
            fs::create_dir_all(home.join(".claude")).unwrap();
            if let Some(body) = login {
                fs::write(
                    home.join(".claude")
                        .join(format!(".{}.json", "credentials")),
                    body,
                )
                .unwrap();
            }
            Rig { home }
        }

        fn env(&self, now: u64, enabled: bool, force: bool) -> Env {
            Env {
                now_ms: now,
                home: self.home.clone(),
                claude_config_dir: None,
                state_file: self.home.join("state").join(state::FILE),
                enabled,
                force,
            }
        }

        fn saved(&self) -> Option<Record> {
            state::read(&self.home.join("state").join(state::FILE))
        }
    }

    fn good_login() -> String {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"TOKEN-XYZ","refreshToken":"REFRESH-XYZ",
                "expiresAt":{},"refreshTokenExpiresAt":{},"subscriptionType":"pro"}}}}"#,
            NOW + 3_600_000,
            NOW + 86_400_000
        )
    }

    fn ok(calls: &Cell<u32>) -> Counting<'_> {
        Counting {
            calls,
            reply: Ok(BODY.to_owned()),
        }
    }

    fn err(calls: &Cell<u32>, e: FetchError) -> Counting<'_> {
        Counting {
            calls,
            reply: Err(e),
        }
    }

    #[test]
    fn switched_off_reads_nothing_and_forgets_the_numbers() {
        let rig = Rig::new("off", Some(&good_login()));
        // Earlier numbers exist.
        let calls = Cell::new(0);
        refresh(&rig.env(NOW, true, true), &ok(&calls));
        assert_eq!(rig.saved().unwrap().windows.len(), 2);
        // Now off: the login file is removed to prove it is not even opened.
        fs::remove_file(
            rig.home
                .join(".claude")
                .join(format!(".{}.json", "credentials")),
        )
        .unwrap();
        let record = refresh(&rig.env(NOW + 1, false, true), &ok(&calls));
        assert_eq!(record.status, Status::Disabled);
        assert!(record.windows.is_empty() && !record.enabled);
        assert_eq!(calls.get(), 1, "no request while off");
        assert_eq!(rig.saved().unwrap().status, Status::Disabled);
    }

    #[test]
    fn a_success_writes_windows_and_the_plan() {
        let rig = Rig::new("ok", Some(&good_login()));
        let calls = Cell::new(0);
        let record = refresh(&rig.env(NOW, true, false), &ok(&calls));
        assert_eq!(record.status, Status::Ok);
        assert_eq!(record.plan, "Pro");
        assert_eq!(record.windows.len(), 2);
        assert_eq!(
            (record.fetched_at_ms, record.probed_at_ms, record.failures),
            (NOW, NOW, 0)
        );
        assert!(!record.stale);
        assert_eq!(rig.saved(), Some(record));
    }

    #[test]
    fn no_login_is_waiting_for_auth_and_keeps_old_numbers() {
        let rig = Rig::new("nologin", None);
        let calls = Cell::new(0);
        let record = refresh(&rig.env(NOW, true, true), &ok(&calls));
        assert_eq!(record.status, Status::WaitingForAuth);
        assert!(record.windows.is_empty());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn a_lapsed_access_token_is_paused_or_expired_by_the_refresh_token() {
        let lapsed = |refresh: u64| {
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"t","expiresAt":{},"refreshTokenExpiresAt":{}}}}}"#,
                NOW - 1,
                refresh
            )
        };
        let rig = Rig::new("paused", Some(&lapsed(NOW + 1000)));
        let calls = Cell::new(0);
        assert_eq!(
            refresh(&rig.env(NOW, true, true), &ok(&calls)).status,
            Status::Paused
        );
        let rig = Rig::new("expired", Some(&lapsed(NOW - 1)));
        assert_eq!(
            refresh(&rig.env(NOW, true, true), &ok(&calls)).status,
            Status::SignInExpired
        );
        assert_eq!(calls.get(), 0, "an expired token is never sent");
    }

    #[test]
    fn the_interval_absorbs_repeats_but_force_skips_it() {
        let rig = Rig::new("interval", Some(&good_login()));
        let calls = Cell::new(0);
        refresh(&rig.env(NOW, true, false), &ok(&calls));
        refresh(&rig.env(NOW + 5_000, true, false), &ok(&calls));
        assert_eq!(calls.get(), 1);
        refresh(&rig.env(NOW + 5_000, true, true), &ok(&calls));
        assert_eq!(calls.get(), 2);
        // The forced request at +5 s counts as the last one.
        refresh(
            &rig.env(NOW + 5_000 + MIN_PROBE_MS - 1, true, false),
            &ok(&calls),
        );
        assert_eq!(calls.get(), 2);
        refresh(
            &rig.env(NOW + 5_000 + MIN_PROBE_MS, true, false),
            &ok(&calls),
        );
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn a_failure_keeps_the_last_numbers_stale_and_zeroes_elapsed_windows() {
        let rig = Rig::new("stale", Some(&good_login()));
        let calls = Cell::new(0);
        refresh(&rig.env(NOW, true, true), &ok(&calls));
        // Two hours later the 5-hour window (reset 2027-01-15T10:00Z) is long past.
        let later = parse_rfc("2027-01-15T12:00:00Z");
        let login = format!(
            r#"{{"claudeAiOauth":{{"accessToken":"t","expiresAt":{},"refreshTokenExpiresAt":{}}}}}"#,
            later + 1_000_000,
            later + 9_000_000
        );
        fs::write(
            rig.home
                .join(".claude")
                .join(format!(".{}.json", "credentials")),
            login,
        )
        .unwrap();
        let record = refresh(&rig.env(later, true, true), &err(&calls, FetchError::Other));
        assert_eq!(record.status, Status::Unavailable);
        assert!(record.stale);
        assert_eq!(
            record.windows[0].percent, 0.0,
            "the elapsed window reads zero"
        );
        assert_eq!(record.windows[1].percent, 40.0);
        assert_eq!(record.failures, 1);
        assert_eq!(record.next_at_ms, later + 60_000);
    }

    fn parse_rfc(text: &str) -> u64 {
        crate::time::parse_rfc3339_ms(text).unwrap()
    }

    #[test]
    fn the_backoff_doubles_to_an_hour_and_force_does_not_overrule_it() {
        assert_eq!(
            [1, 2, 3, 4, 5, 6, 7, 20].map(backoff_ms),
            [
                60_000, 120_000, 240_000, 480_000, 960_000, 1_920_000, 3_600_000, 3_600_000
            ]
        );
        let rig = Rig::new("backoff", Some(&good_login()));
        let calls = Cell::new(0);
        let r1 = refresh(&rig.env(NOW, true, true), &err(&calls, FetchError::Other));
        assert_eq!((r1.failures, r1.next_at_ms), (1, NOW + 60_000));
        // Inside the backoff nothing is asked, even forced.
        refresh(
            &rig.env(NOW + 10_000, true, true),
            &err(&calls, FetchError::Other),
        );
        assert_eq!(calls.get(), 1);
        let r2 = refresh(
            &rig.env(NOW + 60_000, true, true),
            &err(&calls, FetchError::Other),
        );
        assert_eq!(
            (calls.get(), r2.failures, r2.next_at_ms),
            (2, 2, NOW + 60_000 + 120_000)
        );
        // A success clears it.
        let r3 = refresh(&rig.env(NOW + 200_000, true, true), &ok(&calls));
        assert_eq!((r3.status, r3.failures, r3.next_at_ms), (Status::Ok, 0, 0));
    }

    #[test]
    fn retry_after_is_honoured_and_a_recent_429_still_reads_current() {
        let rig = Rig::new("429", Some(&good_login()));
        let calls = Cell::new(0);
        refresh(&rig.env(NOW, true, true), &ok(&calls));
        let r = refresh(
            &rig.env(NOW + 20_000, true, true),
            &err(
                &calls,
                FetchError::Status {
                    code: 429,
                    retry_after_secs: Some(300),
                },
            ),
        );
        assert_eq!(r.status, Status::RateLimited);
        assert_eq!(r.next_at_ms, NOW + 20_000 + 300_000);
        assert!(!r.stale, "measured moments ago: still current");
        assert_eq!(r.failures, 1);
        // Measured long ago: stale.
        let rig = Rig::new("429-old", Some(&good_login()));
        refresh(&rig.env(NOW, true, true), &ok(&calls));
        let r = refresh(
            &rig.env(NOW + CURRENT_ENOUGH_MS + 1, true, true),
            &err(
                &calls,
                FetchError::Status {
                    code: 429,
                    retry_after_secs: None,
                },
            ),
        );
        assert!(r.stale);
    }

    #[test]
    fn transport_failures_retry_soon_and_do_not_count() {
        let rig = Rig::new("transport", Some(&good_login()));
        let calls = Cell::new(0);
        let r = refresh(
            &rig.env(NOW, true, true),
            &err(&calls, FetchError::Transport),
        );
        assert_eq!(
            (r.status, r.failures, r.next_at_ms),
            (Status::Unreachable, 0, NOW + TRANSPORT_RETRY_MS)
        );
    }

    #[test]
    fn a_refused_sign_in_says_so_and_waits_a_long_time() {
        let rig = Rig::new("refused", Some(&good_login()));
        let calls = Cell::new(0);
        for code in [401u16, 403] {
            let r = refresh(
                &rig.env(NOW, true, true),
                &err(
                    &calls,
                    FetchError::Status {
                        code,
                        retry_after_secs: None,
                    },
                ),
            );
            assert_eq!(
                (r.status, r.failures, r.next_at_ms),
                (Status::SignInExpired, 0, NOW + REFUSED_RETRY_MS)
            );
            let _ = fs::remove_dir_all(rig.home.join("state"));
        }
    }

    #[test]
    fn a_person_who_signed_in_again_can_force_past_the_refused_wait() {
        let rig = Rig::new("resign", Some(&good_login()));
        let calls = Cell::new(0);
        let refused = FetchError::Status {
            code: 401,
            retry_after_secs: None,
        };
        refresh(&rig.env(NOW, true, true), &err(&calls, refused));
        assert_eq!(calls.get(), 1);
        // Not forced: the wait stands.
        refresh(&rig.env(NOW + 60_000, true, false), &ok(&calls));
        assert_eq!(calls.get(), 1);
        // Forced: asked again, and a good answer clears it.
        let r = refresh(&rig.env(NOW + 60_000, true, true), &ok(&calls));
        assert_eq!((calls.get(), r.status), (2, Status::Ok));
        // Other backoffs are never overruled, forced or not.
        refresh(
            &rig.env(NOW + 70_000, true, true),
            &err(&calls, FetchError::Other),
        );
        refresh(&rig.env(NOW + 71_000, true, true), &ok(&calls));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn a_huge_retry_after_is_cut_to_an_hour_and_never_overflows() {
        let rig = Rig::new("huge", Some(&good_login()));
        let calls = Cell::new(0);
        let r = refresh(
            &rig.env(NOW, true, true),
            &err(
                &calls,
                FetchError::Status {
                    code: 429,
                    retry_after_secs: Some(u64::MAX),
                },
            ),
        );
        assert_eq!(r.next_at_ms, NOW + MAX_BACKOFF_MS);
    }

    #[test]
    fn an_unknown_shape_is_format_changed_never_a_crash() {
        let rig = Rig::new("shape", Some(&good_login()));
        let calls = Cell::new(0);
        let r = refresh(
            &rig.env(NOW, true, true),
            &Counting {
                calls: &calls,
                reply: Ok("{\"hello\":1}".to_owned()),
            },
        );
        assert_eq!((r.status, r.failures), (Status::FormatChanged, 1));
    }

    #[test]
    fn the_token_never_reaches_the_record_the_file_or_a_debug_print() {
        let rig = Rig::new("leak", Some(&good_login()));
        let calls = Cell::new(0);
        let mut seen = vec![];
        for reply in [
            Ok(BODY.to_owned()),
            Err(FetchError::Other),
            Err(FetchError::Status {
                code: 429,
                retry_after_secs: Some(1),
            }),
            Err(FetchError::Transport),
        ] {
            let record = refresh(
                &rig.env(NOW, true, true),
                &Counting {
                    calls: &calls,
                    reply,
                },
            );
            seen.push(format!("{record:?}"));
            seen.push(record.to_json());
            seen.push(fs::read_to_string(rig.home.join("state").join(state::FILE)).unwrap());
            let _ = fs::remove_dir_all(rig.home.join("state"));
        }
        for text in seen {
            assert!(
                !text.contains("TOKEN-XYZ") && !text.contains("REFRESH-XYZ"),
                "{text}"
            );
        }
        let _ = Path::new("");
    }
}
