//! The record the panel reads: numbers and fixed words, nothing else.
//!
//! `$XDG_STATE_HOME/vexos-ai/usage.json`, mode 0600, replaced atomically. It
//! never holds a token, a header, an email or any text from the other side
//! beyond a cleaned model label. The extension reads it and draws it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::parse::Window;

pub const SCHEMA: u64 = 1;
/// The file, relative to the state directory.
pub const FILE: &str = "vexos-ai/usage.json";

/// What the panel should say. The words are the panel's own, one fixed
/// sentence for each, so the same situation reads the same way every time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Healthy, or a failed check that kept the last good numbers.
    Ok,
    /// Usage is switched off. Nothing was read.
    Disabled,
    /// No sign-in to ask with.
    WaitingForAuth,
    /// The access token lapsed; Claude Code renews it the next time it runs.
    Paused,
    /// The sign-in was refused or its refresh token lapsed.
    SignInExpired,
    /// The check failed and there is nothing to show.
    Unavailable,
    /// Anthropic asked us to slow down.
    RateLimited,
    /// Nothing could be reached.
    Unreachable,
    /// The answer was not the shape we know.
    FormatChanged,
    /// Signed in, but this sign-in has no plan limits (an API key).
    NoPlan,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Disabled => "disabled",
            Status::WaitingForAuth => "waiting-for-auth",
            Status::Paused => "paused",
            Status::SignInExpired => "signin-expired",
            Status::Unavailable => "unavailable",
            Status::RateLimited => "rate-limited",
            Status::Unreachable => "unreachable",
            Status::FormatChanged => "format-changed",
            Status::NoPlan => "no-plan",
        }
    }

    pub fn parse(text: &str) -> Option<Status> {
        [
            Status::Ok,
            Status::Disabled,
            Status::WaitingForAuth,
            Status::Paused,
            Status::SignInExpired,
            Status::Unavailable,
            Status::RateLimited,
            Status::Unreachable,
            Status::FormatChanged,
            Status::NoPlan,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
    }
}

/// Another agent's limits, kept beside Claude's (the top-level fields are
/// Claude's). Present only when that agent is installed and plan limits are on.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub status: Status,
    pub plan: String,
    pub windows: Vec<Window>,
    /// The windows are older than the last check: it failed and they were kept.
    pub stale: bool,
    /// When the numbers were last measured, in ms; 0 when never.
    pub fetched_at_ms: u64,
    /// Not before this time is another request made, in ms.
    pub next_at_ms: u64,
}

fn windows_json(windows: &[Window]) -> Vec<Value> {
    windows
        .iter()
        .map(|w| {
            json!({
                "id": w.id,
                "label": w.label,
                "percent": w.percent,
                "resetsAt": w.resets_at_ms,
            })
        })
        .collect()
}

fn windows_from(value: &Value) -> Option<Vec<Window>> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|w| {
                Some(Window {
                    id: w.get("id")?.as_str()?.to_owned(),
                    label: w.get("label")?.as_str()?.to_owned(),
                    percent: w.get("percent")?.as_f64()?,
                    resets_at_ms: w.get("resetsAt").and_then(Value::as_u64).unwrap_or(0),
                })
            })
            .collect(),
    )
}

impl Source {
    fn to_json(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "plan": self.plan,
            "stale": self.stale,
            "fetchedAt": self.fetched_at_ms,
            "nextAt": self.next_at_ms,
            "windows": windows_json(&self.windows),
        })
    }

    fn from_json(v: &Value) -> Option<Source> {
        let num = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Some(Source {
            status: Status::parse(v.get("status")?.as_str()?)?,
            plan: v
                .get("plan")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            windows: windows_from(v.get("windows")?)?,
            stale: v.get("stale").and_then(Value::as_bool).unwrap_or(false),
            fetched_at_ms: num("fetchedAt"),
            next_at_ms: num("nextAt"),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// When this file was written, in ms.
    pub updated_at_ms: u64,
    /// When the numbers were last measured by Anthropic, in ms; 0 when never.
    pub fetched_at_ms: u64,
    /// When a request was last made, in ms; 0 when never.
    pub probed_at_ms: u64,
    /// Not before this time is another request made (a backoff), in ms.
    pub next_at_ms: u64,
    /// Failures in a row that count toward the backoff.
    pub failures: u32,
    pub enabled: bool,
    pub status: Status,
    pub plan: String,
    pub windows: Vec<Window>,
    /// The windows are older than the last check: it failed and they were kept.
    pub stale: bool,
    /// Codex's limits, when Codex is installed.
    pub codex: Option<Source>,
}

impl Record {
    pub fn empty() -> Record {
        Record {
            updated_at_ms: 0,
            fetched_at_ms: 0,
            probed_at_ms: 0,
            next_at_ms: 0,
            failures: 0,
            enabled: false,
            status: Status::Disabled,
            plan: String::new(),
            windows: Vec::new(),
            stale: false,
            codex: None,
        }
    }

    pub fn to_json(&self) -> String {
        let mut value = json!({
            "schema": SCHEMA,
            "updatedAt": self.updated_at_ms,
            "fetchedAt": self.fetched_at_ms,
            "probedAt": self.probed_at_ms,
            "nextAt": self.next_at_ms,
            "failures": self.failures,
            "enabled": self.enabled,
            "status": self.status.as_str(),
            "plan": self.plan,
            "stale": self.stale,
            "windows": windows_json(&self.windows),
        });
        if let Some(codex) = &self.codex {
            value["codex"] = codex.to_json();
        }
        let mut text = value.to_string();
        text.push('\n');
        text
    }

    /// Reads a record back, leniently: a file from another version, or damaged,
    /// is `None` and the caller starts again.
    pub fn from_json(text: &str) -> Option<Record> {
        let v: Value = serde_json::from_str(text).ok()?;
        if v.get("schema")?.as_u64()? != SCHEMA {
            return None;
        }
        let num = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let windows = windows_from(v.get("windows")?)?;
        Some(Record {
            updated_at_ms: num("updatedAt"),
            fetched_at_ms: num("fetchedAt"),
            probed_at_ms: num("probedAt"),
            next_at_ms: num("nextAt"),
            failures: u32::try_from(num("failures")).unwrap_or(u32::MAX),
            enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            status: Status::parse(v.get("status")?.as_str()?)?,
            plan: v
                .get("plan")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            windows,
            stale: v.get("stale").and_then(Value::as_bool).unwrap_or(false),
            // A damaged codex section is no section; Claude's numbers still count.
            codex: v.get("codex").and_then(Source::from_json),
        })
    }
}

/// Windows whose reset time has passed come back at 0%, never dropped: a reset
/// allowance is untouched, not unknown. A window with no reset time is kept as
/// it is. Reset times are only ever the service's own, never computed.
pub fn zero_elapsed(windows: &[Window], now_ms: u64) -> Vec<Window> {
    windows
        .iter()
        .map(|w| {
            if w.resets_at_ms > 0 && w.resets_at_ms <= now_ms {
                Window {
                    percent: 0.0,
                    resets_at_ms: 0,
                    ..w.clone()
                }
            } else {
                w.clone()
            }
        })
        .collect()
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub fn state_home(home: &Path, xdg_state_home: Option<&str>) -> PathBuf {
    match xdg_state_home {
        Some(dir) if !dir.is_empty() && Path::new(dir).is_absolute() => PathBuf::from(dir),
        _ => home.join(".local").join("state"),
    }
}

pub fn read(file: &Path) -> Option<Record> {
    Record::from_json(&std::fs::read_to_string(file).ok()?)
}

/// Writes the record to a temporary file beside the target (0600 on Unix) and
/// renames it into place, so a reader never sees half a file.
pub fn write(file: &Path, record: &Record) -> std::io::Result<()> {
    use std::io::Write;
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // Unique per write, so two writers can never share (and damage) one file.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".usage.json.{}.{n}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut out| {
        out.write_all(record.to_json().as_bytes())?;
        out.sync_all()
    });
    // Whatever failed, no temporary file is left behind.
    written
        .and_then(|()| std::fs::rename(&tmp, file))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, percent: f64, resets: u64) -> Window {
        Window {
            id: id.to_owned(),
            label: id.to_owned(),
            percent,
            resets_at_ms: resets,
        }
    }

    fn sample() -> Record {
        Record {
            updated_at_ms: 5,
            fetched_at_ms: 4,
            probed_at_ms: 4,
            next_at_ms: 9,
            failures: 2,
            enabled: true,
            status: Status::RateLimited,
            plan: "Max 5x".to_owned(),
            windows: vec![window("session", 12.5, 100), window("weekly", 40.0, 0)],
            stale: true,
            codex: Some(Source {
                status: Status::Ok,
                plan: "Plus".to_owned(),
                windows: vec![window("session", 25.0, 200)],
                stale: false,
                fetched_at_ms: 6,
                next_at_ms: 0,
            }),
        }
    }

    #[test]
    fn a_record_round_trips() {
        let record = sample();
        assert_eq!(Record::from_json(&record.to_json()), Some(record));
    }

    #[test]
    fn every_status_round_trips_and_names_are_distinct() {
        let all = [
            Status::Ok,
            Status::Disabled,
            Status::WaitingForAuth,
            Status::Paused,
            Status::SignInExpired,
            Status::Unavailable,
            Status::RateLimited,
            Status::Unreachable,
            Status::FormatChanged,
            Status::NoPlan,
        ];
        for status in all {
            assert_eq!(Status::parse(status.as_str()), Some(status));
        }
        let mut names: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
        assert_eq!(Status::parse("bogus"), None);
    }

    #[test]
    fn a_foreign_or_damaged_file_is_none() {
        assert_eq!(Record::from_json(""), None);
        assert_eq!(Record::from_json("{}"), None);
        assert_eq!(
            Record::from_json(r#"{"schema":2,"windows":[],"status":"ok"}"#),
            None
        );
        assert_eq!(
            Record::from_json(r#"{"schema":1,"windows":[],"status":"nope"}"#),
            None
        );
        assert_eq!(Record::from_json(r#"{"schema":1,"status":"ok"}"#), None);
    }

    #[test]
    fn the_json_has_exactly_the_documented_keys() {
        let value: Value = serde_json::from_str(&sample().to_json()).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "codex",
                "enabled",
                "failures",
                "fetchedAt",
                "nextAt",
                "plan",
                "probedAt",
                "schema",
                "stale",
                "status",
                "updatedAt",
                "windows"
            ]
        );
        // Codex's section has numbers and fixed words too, and no more.
        let mut inner: Vec<&str> = value["codex"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        inner.sort_unstable();
        assert_eq!(
            inner,
            ["fetchedAt", "nextAt", "plan", "stale", "status", "windows"]
        );
        // Without Codex there is no section at all.
        let mut bare = sample();
        bare.codex = None;
        let value: Value = serde_json::from_str(&bare.to_json()).unwrap();
        assert!(value.get("codex").is_none());
    }

    #[test]
    fn a_damaged_codex_section_leaves_claudes_numbers() {
        let mut record = sample();
        let mut value: Value = serde_json::from_str(&record.to_json()).unwrap();
        value["codex"] = serde_json::json!({"status": "bogus", "windows": []});
        record.codex = None;
        assert_eq!(Record::from_json(&value.to_string()), Some(record));
    }

    #[test]
    fn elapsed_windows_read_zero() {
        let windows = [
            window("a", 80.0, 100),
            window("b", 80.0, 200),
            window("c", 80.0, 0),
        ];
        let zeroed = zero_elapsed(&windows, 150);
        assert_eq!((zeroed[0].percent, zeroed[0].resets_at_ms), (0.0, 0));
        assert_eq!((zeroed[1].percent, zeroed[1].resets_at_ms), (80.0, 200));
        assert_eq!((zeroed[2].percent, zeroed[2].resets_at_ms), (80.0, 0));
        assert_eq!(
            zero_elapsed(&windows, 100)[0].percent,
            0.0,
            "at the reset instant it has reset"
        );
    }

    #[test]
    fn the_state_home_follows_xdg_when_absolute() {
        let home = Path::new("/home/u");
        // An absolute path on whatever platform the test runs on.
        let absolute = std::env::temp_dir().join("state");
        assert_eq!(state_home(home, absolute.to_str()), absolute);
        assert_eq!(
            state_home(home, Some("relative")),
            home.join(".local/state")
        );
        assert_eq!(state_home(home, Some("")), home.join(".local/state"));
        assert_eq!(state_home(home, None), home.join(".local/state"));
    }

    #[test]
    fn writing_is_atomic_and_private() {
        let dir = std::env::temp_dir().join(format!("vexos-usage-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join(FILE);
        write(&file, &sample()).unwrap();
        assert_eq!(read(&file), Some(sample()));
        // No temporary file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["usage.json"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
