//! The usage line under the header: the 5-hour limit from Claude Code's
//! `/usage` report, read by a strict parser. It is display text only: it is
//! never sent, logged or stored, and nothing about modes, agents or approvals
//! depends on it. The label is built from validated pieces (a number and a
//! short ASCII text), never from markup in the reply.
//!
//! The report comes from `claude-agent-acp`, which builds it from an SDK call
//! Anthropic marks experimental (`usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`).
//! If its wording changes, the parser finds nothing and the line says
//! "unknown"; it never guesses.

use std::time::{Duration, Instant};

/// Under `$XDG_CONFIG_HOME`, read only, in the style of the notification
/// settings. One `key = true|false` per line:
///
/// ```text
/// usage-on-launch = false
/// ```
pub const PANEL_SETTINGS_FILE: &str = "vexos-ai/panel.conf";

/// The automatic `/usage` turn locks the input for at most this long; after
/// that the input opens and the turn keeps running.
pub const USAGE_LOCK: Duration = Duration::from_secs(15);

/// The reset text is at most this many characters.
const MAX_RESETS: usize = 40;

/// What the panel's settings file says. A missing file, an unknown key or a
/// bad value leaves the default: `/usage` runs once at launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelSettings {
    pub usage_on_launch: bool,
}

impl Default for PanelSettings {
    fn default() -> Self {
        PanelSettings {
            usage_on_launch: true,
        }
    }
}

impl PanelSettings {
    pub fn parse(text: &str) -> PanelSettings {
        let mut s = PanelSettings::default();
        for line in text.lines() {
            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };
            let value = match value.trim() {
                "true" => true,
                "false" => false,
                _ => continue,
            };
            if key.trim() == "usage-on-launch" {
                s.usage_on_launch = value;
            }
        }
        s
    }
}

/// The 5-hour limit as the report states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiveHour {
    /// Percent used, 0 to 100.
    pub percent: u8,
    /// The reset time as written after "Resets", with U+202F and U+00A0
    /// turned into ASCII spaces: ASCII letters, digits, spaces, commas,
    /// colons and periods, at most 40 characters.
    pub resets: String,
}

/// The 5-hour limit from one `/usage` reply. Only the first line that names
/// the 5-hour limit counts; if that line is not exactly
/// `5-hour limit — <0..=100>% · Resets <text>` (bold markers aside), the
/// answer is `None`. Nothing is guessed, converted or computed.
pub fn five_hour(reply: &str) -> Option<FiveHour> {
    let line = reply
        .lines()
        .map(|line| line.replace("**", ""))
        .find(|line| line.trim_start().starts_with("5-hour limit"))?;
    let rest = line.trim().strip_prefix("5-hour limit — ")?;
    let (percent, rest) = rest.split_once("% · Resets ")?;
    if percent.is_empty() || percent.len() > 3 || !percent.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let percent: u8 = percent.parse().ok().filter(|&p| p <= 100)?;
    // Node's date formatter puts U+202F (or U+00A0) before "AM"/"PM": those
    // two, and only those, become an ASCII space, one for one. Then the text
    // is validated as it will be shown.
    let resets: String = rest
        .trim()
        .chars()
        .map(|c| match c {
            '\u{202f}' | '\u{a0}' => ' ',
            c => c,
        })
        .collect();
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, ' ' | ',' | ':' | '.');
    if resets.is_empty() || resets.chars().count() > MAX_RESETS || !resets.chars().all(allowed) {
        return None;
    }
    Some(FiveHour { percent, resets })
}

/// "updated just now", or "updated N min ago", from a monotonic clock.
pub fn freshness(at: Instant, now: Instant) -> String {
    let minutes = now.saturating_duration_since(at).as_secs() / 60;
    if minutes == 0 {
        "updated just now".to_owned()
    } else {
        format!("updated {minutes} min ago")
    }
}

/// Why the usage line can't be clicked now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disabled {
    NotReady,
    Loading,
    TurnRunning,
    ApprovalWaiting,
}

/// The usage line, as the window shows it. Read-only; a click goes to
/// [`crate::Conversation::check_usage`] and nowhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageButton {
    pub visible: bool,
    pub enabled: bool,
    pub wide: String,
    pub narrow: String,
    pub tooltip: &'static str,
}

pub const CLICK_TOOLTIP: &str = "Click to show the full usage report in the chat";

pub fn tooltip(disabled: Option<Disabled>) -> &'static str {
    match disabled {
        None => CLICK_TOOLTIP,
        Some(Disabled::NotReady) => "Not available: the session is not ready",
        Some(Disabled::Loading) => "Not available while usage is loading",
        Some(Disabled::TurnRunning) => "Not available while a turn is running",
        Some(Disabled::ApprovalWaiting) => "Not available while an approval is waiting",
    }
}

/// What the line shows: loading, unknown, or the 5-hour figure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Loading,
    Unknown,
    Known(FiveHour, Instant),
}

impl Shown {
    /// The wide and narrow labels.
    pub(crate) fn labels(&self, now: Instant) -> (String, String) {
        match self {
            Shown::Loading => ("Usage: loading".to_owned(), "Usage: loading".to_owned()),
            Shown::Unknown => ("Usage: unknown".to_owned(), "Usage: unknown".to_owned()),
            Shown::Known(f, at) => (
                format!(
                    "5-hour {}% · resets {} · {}",
                    f.percent,
                    f.resets,
                    freshness(*at, now)
                ),
                format!("{}%", f.percent),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &str = "5-hour limit — 25% · Resets Oct 27, 1:40 PM CDT";

    fn fh(percent: u8, resets: &str) -> Option<FiveHour> {
        Some(FiveHour {
            percent,
            resets: resets.into(),
        })
    }

    /// The report as `claude-agent-acp` 0.88.0 writes it (`usage-markdown.ts`).
    fn report(five: &str) -> String {
        format!(
            "## Usage\n\n> Claude Max subscription usage\n\n### Limits\n\n{five}\n\n\
             `█████░░░░░░░░░░░░░░░`\n\n\
             **Weekly · all models** — **49%** · Resets Oct 30, 6:00 PM CDT\n\n\
             `██████████░░░░░░░░░░`\n\n---\n\n### This session\n\n\
             | Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 1m 2s |\n\n\
             ### What’s using your limits?\n\n> Approximate"
        )
    }

    #[test]
    fn the_known_line_parses() {
        assert_eq!(five_hour(KNOWN), fh(25, "Oct 27, 1:40 PM CDT"));
        // As markdown, inside the whole report.
        let md = report("**5-hour limit** — **25%** · Resets Oct 27, 1:40 PM CDT");
        assert_eq!(five_hour(&md), fh(25, "Oct 27, 1:40 PM CDT"));
    }

    #[test]
    fn only_the_five_hour_line_counts() {
        let weekly_first = "Weekly · all models — 49% · Resets Oct 30, 6:00 PM CDT\n\
                            Weekly limit — 70% · Resets Oct 30, 6:00 PM CDT\n\
                            Weekly · Opus — 70% · Resets Oct 30, 6:00 PM CDT\n\
                            Extra usage — 10% · Resets Nov 1\n";
        assert_eq!(five_hour(weekly_first), None);
        assert_eq!(
            five_hour(&format!("{weekly_first}{KNOWN}")),
            fh(25, "Oct 27, 1:40 PM CDT")
        );
    }

    #[test]
    fn a_missing_five_hour_line_is_none() {
        assert_eq!(five_hour(&report("")), None);
        assert_eq!(five_hour(""), None);
        assert_eq!(five_hour("The agent finished without a reply."), None);
    }

    #[test]
    fn percent_must_be_a_whole_number_from_0_to_100() {
        let line = |p: &str| format!("5-hour limit — {p}% · Resets Oct 27, 1:40 PM CDT");
        assert_eq!(five_hour(&line("0")), fh(0, "Oct 27, 1:40 PM CDT"));
        assert_eq!(five_hour(&line("100")), fh(100, "Oct 27, 1:40 PM CDT"));
        for bad in [
            "101", "-5", "25.5", "2,5", "abc", "", " 25", "+25", "0x19", "1000", "２５",
        ] {
            assert_eq!(five_hour(&line(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn reset_text_must_be_short_plain_ascii() {
        let line = |r: &str| format!("5-hour limit — 25% · Resets {r}");
        assert_eq!(
            five_hour(&line("  Oct 27, 1:40 PM CDT  ")),
            fh(25, "Oct 27, 1:40 PM CDT")
        );
        assert_eq!(five_hour(&line(&"9".repeat(40))), fh(25, &"9".repeat(40)));
        for bad in [
            "<b>Oct 27</b>",
            "Oct 27 <span foreground=\"red\">x</span>",
            "[Oct 27](javascript:alert(1))",
            "`Oct 27`",
            "Oct 27\u{7}",
            "Oct 27\t1:40",
            "Oct 27\u{1b}[31m",
            &"9".repeat(41),
            "",
            // Lookalikes: Cyrillic O, a fullwidth digit, a ratio sign for the colon.
            "\u{41e}ct 27, 1:40 PM",
            "Oct ２7, 1:40 PM",
            "Oct 27, 1\u{2236}40 PM",
            // Other Unicode spaces stay rejected: only U+202F and U+00A0 map.
            "Oct 27, 1:40\u{2009}PM CDT",
            "Oct 27, 1:40\u{2003}PM CDT",
            "Oct 27,\u{3000}1:40 PM",
            "Oct 27\u{200b}, 1:40 PM",
            // A mapped space doesn't let anything else in.
            "1:40\u{202f}PM GMT+2",
            "1:40\u{a0}PM <b>",
        ] {
            assert_eq!(five_hour(&line(bad)), None, "{bad:?}");
        }
    }

    /// Node's `Intl.DateTimeFormat` (ICU 72 and later) writes the space before
    /// "PM" as U+202F; some formats use U+00A0. Each becomes one ASCII space.
    #[test]
    fn no_break_spaces_before_am_pm_become_ascii_spaces() {
        let line = |r: &str| format!("5-hour limit — 25% · Resets {r}");
        for nbsp in ['\u{202f}', '\u{a0}'] {
            let parsed = five_hour(&line(&format!("Oct 27, 1:40{nbsp}PM CDT")));
            assert_eq!(parsed, fh(25, "Oct 27, 1:40 PM CDT"), "{nbsp:?}");
            let shown = Shown::Known(parsed.unwrap(), Instant::now());
            let (wide, _) = shown.labels(Instant::now());
            assert!(
                wide.contains("1:40 PM CDT") && !wide.contains(nbsp),
                "{wide:?}"
            );
            // One for one: two in a row stay two spaces.
            assert_eq!(
                five_hour(&line(&format!("9:05{nbsp}{nbsp}AM"))),
                fh(25, "9:05  AM")
            );
        }
        // As markdown, inside the whole report.
        let md = report("**5-hour limit** — **25%** · Resets Oct 27, 1:40\u{202f}PM CDT");
        assert_eq!(five_hour(&md), fh(25, "Oct 27, 1:40 PM CDT"));
    }

    #[test]
    fn the_cap_counts_characters_after_the_mapping() {
        let line = |r: &str| format!("5-hour limit — 25% · Resets {r}");
        // 40 characters (more than 40 bytes before the mapping): accepted.
        let forty = format!("{}\u{202f}PM", "9".repeat(37));
        assert_eq!(forty.chars().count(), 40);
        assert!(forty.len() > 40);
        assert_eq!(
            five_hour(&line(&forty)),
            fh(25, &format!("{} PM", "9".repeat(37)))
        );
        // 41: rejected.
        let forty_one = format!("{}\u{202f}PM", "9".repeat(38));
        assert_eq!(five_hour(&line(&forty_one)), None);
    }

    #[test]
    fn the_first_five_hour_line_wins_even_when_it_is_bad() {
        let two = format!("{KNOWN}\n5-hour limit — 90% · Resets Nov 1, 2:00 PM CDT");
        assert_eq!(five_hour(&two), fh(25, "Oct 27, 1:40 PM CDT"));
        let bad_first = format!("5-hour limit — 250% · Resets x\n{KNOWN}");
        assert_eq!(five_hour(&bad_first), None);
    }

    #[test]
    fn claude_codes_fallback_text_is_none() {
        // Not the adapter's format: no guessing from it.
        for fallback in [
            "Current session\n  █████▌ 25% used\n  Resets 1:40pm (America/Chicago)",
            "5-hour limit: 25% used, resets at 1:40pm",
            "5-hour limit — 25%",
            "Usage is not available for API key users.",
        ] {
            assert_eq!(five_hour(fallback), None, "{fallback:?}");
        }
    }

    #[test]
    fn labels_wide_and_narrow() {
        let at = Instant::now();
        let known = Shown::Known(
            FiveHour {
                percent: 25,
                resets: "Oct 27, 1:40 PM CDT".into(),
            },
            at,
        );
        assert_eq!(
            known.labels(at),
            (
                "5-hour 25% · resets Oct 27, 1:40 PM CDT · updated just now".to_owned(),
                "25%".to_owned()
            )
        );
        assert_eq!(
            Shown::Loading.labels(at),
            ("Usage: loading".to_owned(), "Usage: loading".to_owned())
        );
        assert_eq!(
            Shown::Unknown.labels(at),
            ("Usage: unknown".to_owned(), "Usage: unknown".to_owned())
        );
    }

    #[test]
    fn freshness_counts_whole_minutes() {
        let at = Instant::now();
        let after = |s| at + Duration::from_secs(s);
        assert_eq!(freshness(at, at), "updated just now");
        assert_eq!(freshness(at, after(59)), "updated just now");
        assert_eq!(freshness(at, after(60)), "updated 1 min ago");
        assert_eq!(freshness(at, after(7 * 60 + 59)), "updated 7 min ago");
        assert_eq!(freshness(at, after(125 * 60)), "updated 125 min ago");
        // A clock read before the result says "just now", never a negative age.
        assert_eq!(freshness(after(60), at), "updated just now");
    }

    #[test]
    fn tooltips_say_why() {
        assert_eq!(
            tooltip(None),
            "Click to show the full usage report in the chat"
        );
        for (why, text) in [
            (Disabled::NotReady, "the session is not ready"),
            (Disabled::Loading, "usage is loading"),
            (Disabled::TurnRunning, "a turn is running"),
            (Disabled::ApprovalWaiting, "an approval is waiting"),
        ] {
            assert!(tooltip(Some(why)).contains(text), "{why:?}");
        }
    }

    #[test]
    fn the_setting_defaults_on_and_turns_off() {
        assert!(PanelSettings::default().usage_on_launch);
        assert!(PanelSettings::parse("").usage_on_launch);
        assert!(!PanelSettings::parse("usage-on-launch = false\n").usage_on_launch);
        assert!(PanelSettings::parse("usage-on-launch = no").usage_on_launch);
        assert!(PanelSettings::parse("# usage-on-launch = false").usage_on_launch);
        assert!(PanelSettings::parse("other = false").usage_on_launch);
    }
}
