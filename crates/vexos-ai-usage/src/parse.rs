//! The usage endpoint's answer, as windows of numbers.
//!
//! Anthropic's payload has flat buckets (`five_hour`, `seven_day`) with a
//! `utilization` and a `resets_at`, and a `limits` array whose entries scoped to
//! a model are the only place a model-specific allowance shows up. Every
//! utilization is already a percentage: Claude Code's own `/usage` renders each
//! as `${Math.floor(utilization)}% used`, so 0.6 means 0.6%, not 60%.
//!
//! Nothing here trusts the payload: numbers are checked, text is cut down to a
//! safe alphabet, and an unknown shape is an error, never a panic.

use serde_json::Value;

use crate::time::parse_rfc3339_ms;

/// One allowance: a window of time and how much of it is used.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// "session", "weekly", or "scoped:<label>".
    pub id: String,
    /// What the panel shows: "Session", "Weekly", "Fable Weekly".
    pub label: String,
    /// 0 to 100.
    pub percent: f64,
    /// When the window resets, in ms since the epoch; 0 when unknown.
    pub resets_at_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Not JSON, or not an object.
    NotAnObject,
    /// An object with no window in it: the format changed.
    NoWindows,
}

/// A percentage from a number or a string such as "12.5%". Negative, NaN and
/// infinite values are no percentage. Above 100 is cut to 100.
fn percent_of(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().trim_end_matches('%').trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && n >= 0.0).then_some(n.min(100.0))
}

/// A reset time from an RFC 3339 string or an epoch in seconds or milliseconds.
fn reset_of(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::String(s)) => {
            let s = s.trim();
            if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
                s.parse::<u64>().map_or(0, epoch_to_ms)
            } else {
                parse_rfc3339_ms(s).unwrap_or(0)
            }
        }
        Some(Value::Number(n)) => n.as_u64().map_or(0, epoch_to_ms),
        _ => 0,
    }
}

fn epoch_to_ms(n: u64) -> u64 {
    if n < 1_000_000_000_000 {
        n.saturating_mul(1000)
    } else {
        n
    }
}

/// Text from the payload, cut to what a model name needs: printable ASCII
/// letters, digits, spaces and `()+-._`, at most 40 characters.
fn clean_label(text: &str) -> String {
    text.chars()
        .filter(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ' ' | '(' | ')' | '+' | '-' | '.' | '_')
        })
        .take(40)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// "Weekly", "Monthly" or "Session" from an entry's `kind`, as the flat
/// windows title themselves; "" when it names none.
fn window_title(kind: &str) -> &'static str {
    let kind = kind.to_ascii_lowercase();
    if kind.contains("month") {
        "Monthly"
    } else if kind.contains("week") || kind.contains("day") {
        "Weekly"
    } else if kind.contains("hour") || kind.contains("session") {
        "Session"
    } else {
        ""
    }
}

fn flat(payload: &Value, key: &str, id: &str, label: &str) -> Option<Window> {
    let bucket = payload.get(key).filter(|v| v.is_object())?;
    Some(Window {
        id: id.to_owned(),
        label: label.to_owned(),
        percent: percent_of(bucket.get("utilization")?)?,
        resets_at_ms: reset_of(bucket.get("resets_at")),
    })
}

/// The allowances scoped to one model, from the `limits` array. A model can
/// hold more than one window, so the pair of model and kind makes the key.
fn scoped(payload: &Value) -> Vec<Window> {
    let Some(entries) = payload.get("limits").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<Window> = Vec::new();
    for entry in entries {
        let Some(model) = entry
            .get("scope")
            .and_then(|s| s.get("model"))
            .filter(|m| m.is_object())
        else {
            continue;
        };
        let display = model
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let id = model.get("id").and_then(Value::as_str).unwrap_or("");
        let name = clean_label(if display.is_empty() { id } else { display });
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let Some(percent) = entry.get("percent").and_then(percent_of) else {
            continue;
        };
        let title = window_title(kind);
        let label = if title.is_empty() {
            name
        } else {
            format!("{name} {title}")
        };
        let key = format!("scoped:{label}");
        if out.iter().any(|w| w.id == key) {
            continue;
        }
        out.push(Window {
            id: key,
            label,
            percent,
            resets_at_ms: reset_of(entry.get("resets_at")),
        });
    }
    out
}

/// The windows in an answer: Session, Weekly, then model-scoped ones.
pub fn parse_usage(body: &str) -> Result<Vec<Window>, ParseError> {
    let payload: Value = serde_json::from_str(body).map_err(|_| ParseError::NotAnObject)?;
    if !payload.is_object() {
        return Err(ParseError::NotAnObject);
    }
    let mut windows = Vec::new();
    windows.extend(flat(&payload, "five_hour", "session", "Session"));
    windows.extend(flat(&payload, "seven_day", "weekly", "Weekly"));
    windows.extend(scoped(&payload));
    if windows.is_empty() {
        return Err(ParseError::NoWindows);
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
      "five_hour": {"utilization": 12.5, "resets_at": "2026-10-06T21:00:00+00:00"},
      "seven_day": {"utilization": "40%", "resets_at": 1791320400},
      "seven_day_oauth_apps": {"utilization": 99, "resets_at": null},
      "limits": [
        {"kind":"weekly_scoped","percent":30,"resets_at":"2026-10-10T08:00:00Z",
         "scope":{"model":{"display_name":"Fable","id":"fable-5"}}},
        {"kind":"weekly_scoped","percent":31,"scope":{"model":{"display_name":"Fable"}}},
        {"kind":"five_hour_scoped","percent":5,"scope":{"model":{"id":"opus-5 (1M context)"}}},
        {"kind":"weekly","percent":9,"scope":{"other":1}},
        {"kind":"weekly_scoped","percent":-1,"scope":{"model":{"display_name":"Bad"}}}
      ]
    }"#;

    #[test]
    fn reads_flat_and_scoped_windows() {
        let windows = parse_usage(FULL).unwrap();
        let ids: Vec<&str> = windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "session",
                "weekly",
                "scoped:Fable Weekly",
                "scoped:opus-5 (1M context) Session"
            ]
        );
        assert_eq!(windows[0].percent, 12.5);
        assert_eq!(windows[0].resets_at_ms, 1_791_320_400_000);
        assert_eq!(windows[1].percent, 40.0);
        assert_eq!(windows[1].resets_at_ms, 1_791_320_400_000);
        // The oauth-apps side bucket is never the weekly limit, and the repeat
        // of a model and window is dropped.
        assert_eq!(windows[2].label, "Fable Weekly");
        assert_eq!(windows[2].percent, 30.0);
    }

    #[test]
    fn a_percentage_is_a_percentage_not_a_fraction() {
        let windows = parse_usage(r#"{"five_hour":{"utilization":0.6}}"#).unwrap();
        assert_eq!(windows[0].percent, 0.6);
        assert_eq!(windows[0].resets_at_ms, 0);
    }

    #[test]
    fn over_range_is_cut_and_bad_numbers_are_dropped() {
        let windows =
            parse_usage(r#"{"five_hour":{"utilization":250},"seven_day":{"utilization":-3}}"#)
                .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].percent, 100.0);
        for bad in [
            r#"{"five_hour":{"utilization":"abc"}}"#,
            r#"{"five_hour":{"utilization":null}}"#,
            r#"{"five_hour":{}}"#,
        ] {
            assert_eq!(parse_usage(bad), Err(ParseError::NoWindows), "{bad}");
        }
    }

    #[test]
    fn shapes_that_are_not_usage_are_errors() {
        for (body, expected) in [
            ("", ParseError::NotAnObject),
            ("not json", ParseError::NotAnObject),
            ("[1,2]", ParseError::NotAnObject),
            ("\"x\"", ParseError::NotAnObject),
            ("{}", ParseError::NoWindows),
            (r#"{"limits":"x"}"#, ParseError::NoWindows),
            (r#"{"five_hour":"x"}"#, ParseError::NoWindows),
        ] {
            assert_eq!(parse_usage(body), Err(expected), "{body}");
        }
    }

    #[test]
    fn labels_are_cut_to_a_safe_alphabet() {
        // A right-to-left override, built here so the source file has none.
        let bidi = char::from_u32(0x202e).unwrap();
        let body = format!(
            r#"{{"limits":[{{"kind":"weekly","percent":1,
            "scope":{{"model":{{"display_name":"A<script>\u0000\n{bidi} B"}}}}}}]}}"#
        );
        let windows = parse_usage(&body).unwrap();
        assert_eq!(windows[0].label, "Ascript B Weekly");
        let long = format!(
            r#"{{"limits":[{{"kind":"weekly","percent":1,"scope":{{"model":{{"display_name":"{}"}}}}}}]}}"#,
            "x".repeat(200)
        );
        assert!(parse_usage(&long).unwrap()[0].label.len() <= 40 + " Weekly".len());
    }
}
