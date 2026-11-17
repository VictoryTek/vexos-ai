//! Codex's plan limits, asked of Codex itself.
//!
//! Codex has a supported way to report them: `codex app-server`, a JSON-RPC
//! server on stdin and stdout, answers `account/read` and
//! `account/rateLimits/read`. So unlike Claude's, this source reads no sign-in
//! file and holds no token: Codex uses its own login, and what comes back is
//! cut down to numbers and fixed words before anything is kept. Nothing in this
//! file names Codex's login file, and the account's email is never copied.
//!
//! One short-lived child per request: started, asked three questions, killed.
//! A deadline and a byte cap bound it, so a hung or chatty Codex cannot hold
//! the service or fill its memory.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::parse::Window;

/// How long one request may take, from start to answer.
pub const TIMEOUT: Duration = Duration::from_secs(15);
/// Most bytes read from the child in one request; the answers are a few hundred.
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexError {
    /// There is no `codex` to ask: the panel shows no Codex section.
    NotInstalled,
    /// It did not answer in time, closed early, or answered with an error.
    Failed,
}

/// What Codex said, before it is interpreted.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The `account/read` result.
    pub account: Value,
    /// The `account/rateLimits/read` result; only asked when signed in with a
    /// ChatGPT plan, since an API key or no login has no plan limits.
    pub limits: Option<Value>,
}

/// Something that can ask Codex. The real one starts a process; a test supplies
/// its own.
pub trait Limits {
    fn read(&self) -> Result<Reading, CodexError>;
}

/// For callers that never ask Codex.
pub struct NoCodex;

impl Limits for NoCodex {
    fn read(&self) -> Result<Reading, CodexError> {
        Err(CodexError::NotInstalled)
    }
}

/// `codex app-server`, started by name from the service's PATH.
pub struct AppServer {
    program: OsString,
    timeout: Duration,
}

impl AppServer {
    pub fn new() -> AppServer {
        AppServer::with("codex", TIMEOUT)
    }

    pub fn with(program: impl Into<OsString>, timeout: Duration) -> AppServer {
        AppServer {
            program: program.into(),
            timeout,
        }
    }
}

impl Default for AppServer {
    fn default() -> Self {
        AppServer::new()
    }
}

/// Kills and reaps the child however the request ends.
struct Reaper(Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Limits for AppServer {
    fn read(&self) -> Result<Reading, CodexError> {
        let child = Command::new(&self.program)
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => CodexError::NotInstalled,
                _ => CodexError::Failed,
            })?;
        let mut child = Reaper(child);
        let mut stdin = child.0.stdin.take().ok_or(CodexError::Failed)?;
        let stdout = child.0.stdout.take().ok_or(CodexError::Failed)?;

        // A thread reads, so the deadline holds however the child behaves: when
        // the request ends the child is killed, the pipe closes, the thread ends.
        let (tx, rx) = mpsc::channel::<Value>();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout.take(MAX_BYTES));
            let mut line = String::new();
            loop {
                line.clear();
                match lines.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                if let Ok(value) = serde_json::from_str::<Value>(line.trim())
                    && tx.send(value).is_err()
                {
                    return;
                }
            }
        });

        let deadline = Instant::now() + self.timeout;
        let ask = |stdin: &mut std::process::ChildStdin,
                   id: u64,
                   method: &str,
                   params: Option<Value>|
         -> Result<Value, CodexError> {
            let mut message = json!({ "method": method, "id": id });
            if let Some(params) = params {
                message["params"] = params;
            }
            send(stdin, &message)?;
            answer(&rx, id, deadline)
        };

        ask(
            &mut stdin,
            1,
            "initialize",
            Some(json!({
                "clientInfo": {
                    "name": "vexos_ai_usage",
                    "title": "VexOS usage",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            })),
        )?;
        send(&mut stdin, &json!({ "method": "initialized" }))?;
        let account = ask(&mut stdin, 2, "account/read", Some(json!({})))?;
        let limits = if signed_in_with_a_plan(&account) {
            Some(ask(&mut stdin, 3, "account/rateLimits/read", None)?)
        } else {
            None
        };
        Ok(Reading { account, limits })
    }
}

fn send(stdin: &mut impl Write, message: &Value) -> Result<(), CodexError> {
    let mut line = message.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|_| CodexError::Failed)
}

/// The result of request `id`. Notifications and Codex's own requests (they
/// carry a `method`) are skipped; an error answer, a closed pipe and the
/// deadline are all `Failed`.
fn answer(rx: &mpsc::Receiver<Value>, id: u64, deadline: Instant) -> Result<Value, CodexError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let message = rx.recv_timeout(left).map_err(|_| CodexError::Failed)?;
        if message.get("method").is_some() || message.get("id").and_then(Value::as_u64) != Some(id)
        {
            continue;
        }
        return message.get("result").cloned().ok_or(CodexError::Failed);
    }
}

fn signed_in_with_a_plan(account: &Value) -> bool {
    account
        .pointer("/account/type")
        .and_then(Value::as_str)
        .is_some_and(|t| t == "chatgpt")
}

/// What a reading means for the panel.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// No Codex sign-in.
    SignedOut,
    /// Signed in with an API key (or another provider): no plan limits exist.
    NoPlan,
    Limits {
        plan: String,
        windows: Vec<Window>,
    },
    /// Not the shape we know.
    FormatChanged,
}

pub fn interpret(reading: &Reading) -> Outcome {
    let Some(account) = reading.account.get("account") else {
        return Outcome::FormatChanged;
    };
    if account.is_null() {
        return Outcome::SignedOut;
    }
    if !signed_in_with_a_plan(&reading.account) {
        return Outcome::NoPlan;
    }
    let Some(limits) = &reading.limits else {
        return Outcome::FormatChanged;
    };
    // The `codex` bucket when Codex names its buckets, else the single one.
    let snapshot = limits
        .pointer("/rateLimitsByLimitId/codex")
        .filter(|v| v.is_object())
        .or_else(|| limits.get("rateLimits").filter(|v| v.is_object()));
    let Some(snapshot) = snapshot else {
        return Outcome::FormatChanged;
    };
    let windows: Vec<Window> = ["primary", "secondary"]
        .iter()
        .filter_map(|key| window(snapshot.get(*key)?))
        .collect();
    if windows.is_empty() {
        return Outcome::FormatChanged;
    }
    let plan = account
        .get("planType")
        .or_else(|| snapshot.get("planType"))
        .and_then(Value::as_str)
        .map(plan_label)
        .unwrap_or_default();
    Outcome::Limits { plan, windows }
}

/// One window: `usedPercent`, `windowDurationMins` and `resetsAt` (seconds).
/// A window with no usable percentage is skipped, never guessed.
fn window(value: &Value) -> Option<Window> {
    let used = value.get("usedPercent")?.as_f64()?;
    if !used.is_finite() || used < 0.0 {
        return None;
    }
    let minutes = value
        .get("windowDurationMins")
        .and_then(Value::as_u64)
        .filter(|m| *m > 0);
    let resets_at_ms = value
        .get("resetsAt")
        .and_then(Value::as_u64)
        .and_then(|s| s.checked_mul(1000))
        .unwrap_or(0);
    // The window's length says what it is: 5 hours is the session, 7 days the
    // week. Other lengths are named by their length.
    let (id, label) = match minutes {
        Some(300) => ("session".to_owned(), "Session".to_owned()),
        Some(10_080) => ("weekly".to_owned(), "Weekly".to_owned()),
        Some(m) if m % 1440 == 0 => (format!("window-{m}"), format!("{}-day", m / 1440)),
        Some(m) if m % 60 == 0 => (format!("window-{m}"), format!("{}-hour", m / 60)),
        Some(m) => (format!("window-{m}"), format!("{m}-minute")),
        None => ("window".to_owned(), "Limit".to_owned()),
    };
    Some(Window {
        id,
        label,
        percent: used.min(100.0),
        resets_at_ms,
    })
}

/// "plus" -> "Plus", "pro_lite" -> "Pro lite": a short label from a plan word,
/// cut down to a safe alphabet.
fn plan_label(plan: &str) -> String {
    let cleaned: String = plan
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .take(24)
        .collect();
    let cleaned = cleaned.trim();
    let mut chars = cleaned.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(account: Value, limits: Option<Value>) -> Reading {
        Reading { account, limits }
    }

    fn chatgpt() -> Value {
        json!({"account": {"type": "chatgpt", "email": "someone@example.com", "planType": "plus"},
               "requiresOpenaiAuth": true})
    }

    fn limits() -> Value {
        json!({"rateLimits": {"limitId": "codex",
                  "primary": {"usedPercent": 25, "windowDurationMins": 300, "resetsAt": 1_800_000_000u64},
                  "secondary": {"usedPercent": 61, "windowDurationMins": 10080, "resetsAt": 1_800_500_000u64},
                  "planType": "plus"},
               "rateLimitsByLimitId": null})
    }

    #[test]
    fn a_plan_gives_session_and_weekly_windows() {
        let Outcome::Limits { plan, windows } = interpret(&reading(chatgpt(), Some(limits())))
        else {
            panic!("limits expected");
        };
        assert_eq!(plan, "Plus");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].id, "session");
        assert_eq!(windows[0].label, "Session");
        assert_eq!(windows[0].percent, 25.0);
        assert_eq!(windows[0].resets_at_ms, 1_800_000_000_000);
        assert_eq!(windows[1].id, "weekly");
        assert_eq!(windows[1].percent, 61.0);
    }

    #[test]
    fn the_codex_bucket_wins_when_buckets_are_named() {
        let body = json!({
            "rateLimits": {"primary": {"usedPercent": 1, "windowDurationMins": 300}},
            "rateLimitsByLimitId": {
                "other": {"primary": {"usedPercent": 90, "windowDurationMins": 300}},
                "codex": {"primary": {"usedPercent": 7, "windowDurationMins": 300}}}});
        let Outcome::Limits { windows, .. } = interpret(&reading(chatgpt(), Some(body))) else {
            panic!("limits expected");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].percent, 7.0);
    }

    #[test]
    fn odd_window_lengths_are_named_by_their_length() {
        let body = json!({"rateLimits": {
            "primary": {"usedPercent": 1, "windowDurationMins": 120},
            "secondary": {"usedPercent": 2, "windowDurationMins": 2880}}});
        let Outcome::Limits { windows, .. } = interpret(&reading(chatgpt(), Some(body))) else {
            panic!("limits expected");
        };
        assert_eq!(windows[0].label, "2-hour");
        assert_eq!(windows[1].label, "2-day");
    }

    #[test]
    fn a_missing_window_is_absent_not_zero() {
        let body = json!({"rateLimits": {
            "primary": null,
            "secondary": {"usedPercent": 40, "windowDurationMins": 10080, "resetsAt": null}}});
        let Outcome::Limits { windows, .. } = interpret(&reading(chatgpt(), Some(body))) else {
            panic!("limits expected");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "weekly");
        assert_eq!(windows[0].resets_at_ms, 0);
    }

    #[test]
    fn bad_numbers_are_skipped_and_large_ones_cut() {
        let body = json!({"rateLimits": {
            "primary": {"usedPercent": -5, "windowDurationMins": 300},
            "secondary": {"usedPercent": 250, "windowDurationMins": 10080}}});
        let Outcome::Limits { windows, .. } = interpret(&reading(chatgpt(), Some(body))) else {
            panic!("limits expected");
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].percent, 100.0);
    }

    #[test]
    fn signed_out_an_api_key_and_a_changed_shape_are_told_apart() {
        let out = json!({"account": null, "requiresOpenaiAuth": true});
        assert_eq!(interpret(&reading(out, None)), Outcome::SignedOut);
        let key = json!({"account": {"type": "apiKey"}, "requiresOpenaiAuth": true});
        assert_eq!(interpret(&reading(key, None)), Outcome::NoPlan);
        assert_eq!(
            interpret(&reading(json!({"nope": 1}), None)),
            Outcome::FormatChanged
        );
        assert_eq!(
            interpret(&reading(chatgpt(), Some(json!({"rateLimits": {}})))),
            Outcome::FormatChanged
        );
        assert_eq!(interpret(&reading(chatgpt(), None)), Outcome::FormatChanged);
    }

    #[test]
    fn plan_words_are_cleaned() {
        assert_eq!(plan_label("pro_lite"), "Pro lite");
        assert_eq!(plan_label("plus"), "Plus");
        // A bidi override is dropped with the rest of what is not a letter or digit.
        let bidi = char::from_u32(0x202e).unwrap();
        assert_eq!(plan_label(&format!("<b>x</b>{bidi}")), "Bxb");
        assert_eq!(plan_label(""), "");
        assert!(plan_label(&"a".repeat(100)).len() <= 24);
    }

    #[test]
    fn the_email_is_never_kept() {
        let Outcome::Limits { plan, windows } = interpret(&reading(chatgpt(), Some(limits())))
        else {
            panic!("limits expected");
        };
        let kept = format!("{plan:?}{windows:?}");
        assert!(!kept.contains("someone@example.com"));
    }

    #[test]
    fn nothing_here_names_codexs_login_file() {
        // The name is built so that this test does not contain it.
        let name = format!("{}.{}", "auth", "json");
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    !text.contains(&name),
                    "{} names the login file",
                    path.display()
                );
            }
        }
    }

    /// A pretend `codex` for the process tests: a shell script.
    #[cfg(unix)]
    mod process {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(name: &str, body: &str) -> std::path::PathBuf {
            let dir =
                std::env::temp_dir().join(format!("vexos-codex-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("codex");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        const GOOD: &str = r#"
read line
echo '{"method":"note","params":{}}'
echo '{"id":1,"result":{"userAgent":"x"}}'
read line
read line
echo '{"id":99,"method":"item/approval","params":{}}'
echo '{"id":2,"result":{"account":{"type":"chatgpt","email":"a@b.c","planType":"plus"},"requiresOpenaiAuth":true}}'
read line
echo '{"id":3,"result":{"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300,"resetsAt":1800000000}}}}'
"#;

        #[test]
        fn a_conversation_with_codex_reads_the_limits() {
            let codex = script("good", GOOD);
            let got = AppServer::with(&codex, Duration::from_secs(10))
                .read()
                .unwrap();
            assert!(matches!(interpret(&got), Outcome::Limits { .. }));
        }

        #[test]
        fn signed_out_never_asks_for_limits() {
            let codex = script(
                "out",
                r#"read line
echo '{"id":1,"result":{}}'
read line
read line
echo '{"id":2,"result":{"account":null,"requiresOpenaiAuth":true}}'
sleep 5"#,
            );
            let got = AppServer::with(&codex, Duration::from_secs(10))
                .read()
                .unwrap();
            assert_eq!(got.limits, None);
            assert_eq!(interpret(&got), Outcome::SignedOut);
        }

        #[test]
        fn a_silent_codex_is_cut_off_at_the_deadline() {
            let codex = script("hang", "sleep 30");
            let started = Instant::now();
            let got = AppServer::with(&codex, Duration::from_millis(300)).read();
            assert_eq!(got, Err(CodexError::Failed));
            assert!(started.elapsed() < Duration::from_secs(5));
        }

        #[test]
        fn an_error_answer_fails() {
            let codex = script(
                "err",
                r#"read line
echo '{"id":1,"error":{"code":-32600,"message":"no"}}'
sleep 5"#,
            );
            let got = AppServer::with(&codex, Duration::from_secs(10)).read();
            assert_eq!(got, Err(CodexError::Failed));
        }

        #[test]
        fn a_flood_is_capped_not_stored() {
            let codex = script(
                "flood",
                "yes 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'",
            );
            let got = AppServer::with(&codex, Duration::from_secs(10)).read();
            assert_eq!(got, Err(CodexError::Failed));
        }

        #[test]
        fn no_codex_is_not_installed() {
            let got = AppServer::with("/nonexistent/vexos-codex", Duration::from_secs(1)).read();
            assert_eq!(got, Err(CodexError::NotInstalled));
        }
    }
}
