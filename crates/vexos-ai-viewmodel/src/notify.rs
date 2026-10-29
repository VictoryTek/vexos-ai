//! When to raise a notification. A notification only says that something
//! happened; it carries no log text, and nothing reaches an agent from it.
//! Watchers dedupe and rate-limit here, so they cannot spam.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::diagnose::{Crash, UnitName, core_file};

/// Under `$XDG_CONFIG_HOME`. One `key = true|false` per line, `#` comments:
///
/// ```text
/// failed-units = false
/// coredumps = false
/// ```
pub const SETTINGS_FILE: &str = "vexos-ai/notifications.conf";

/// Which triggers are on. A missing file, an unknown key or a bad value
/// leaves the default: the watchers off. A `rebuild-failed` line from an older
/// file is an unknown key now, and ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub failed_units: bool,
    pub coredumps: bool,
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = match value.trim() {
                "true" => true,
                "false" => false,
                _ => continue,
            };
            match key.trim() {
                "failed-units" => s.failed_units = value,
                "coredumps" => s.coredumps = value,
                _ => {}
            }
        }
        s
    }
}

/// The same key is notified at most once per this many seconds.
pub const COOLDOWN: u64 = 60 * 60;
/// At most [`MAX_PER_WINDOW`] notifications per this many seconds, in all.
pub const WINDOW: u64 = 10 * 60;
pub const MAX_PER_WINDOW: usize = 3;

/// A per-key cooldown and a global cap. Times are seconds on a monotonic clock.
#[derive(Debug, Default)]
struct Limiter {
    last: HashMap<String, u64>,
    recent: VecDeque<u64>,
}

impl Limiter {
    fn allow(&mut self, key: &str, now: u64) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|&t| now.saturating_sub(t) >= WINDOW)
        {
            self.recent.pop_front();
        }
        self.last.retain(|_, t| now.saturating_sub(*t) < COOLDOWN);
        if self.last.contains_key(key) || self.recent.len() >= MAX_PER_WINDOW {
            return false;
        }
        self.last.insert(key.to_owned(), now);
        self.recent.push_back(now);
        true
    }
}

/// Failed units, from a periodic read of systemd's failed list.
#[derive(Debug, Default)]
pub struct FailedUnits {
    /// The valid names failed at the last read; `None` before the first.
    known: Option<HashSet<UnitName>>,
    limiter: Limiter,
}

impl FailedUnits {
    /// The units to notify about, given every name now failed. The first
    /// read only records what is already failed. A unit is new when it was
    /// not failed at the last read; names that fail validation are ignored.
    pub fn observe(&mut self, failed: &[String], now: u64) -> Vec<UnitName> {
        let current: HashSet<UnitName> = failed.iter().filter_map(|n| UnitName::parse(n)).collect();
        let mut new: Vec<UnitName> = match &self.known {
            Some(known) => current.difference(known).cloned().collect(),
            None => Vec::new(),
        };
        new.sort();
        self.known = Some(current);
        new.retain(|unit| self.limiter.allow(&format!("unit:{}", unit.as_str()), now));
        new
    }
}

/// New core files in systemd-coredump's directory, for one uid.
#[derive(Debug)]
pub struct Crashes {
    uid: u32,
    seen: HashSet<String>,
    limiter: Limiter,
}

impl Crashes {
    pub fn new(uid: u32) -> Self {
        Crashes {
            uid,
            seen: HashSet::new(),
            limiter: Limiter::default(),
        }
    }

    /// A crash to notify about, for a file that just appeared. Other users'
    /// files, malformed names and repeats are ignored; a program that keeps
    /// crashing is notified once per cooldown.
    pub fn observe(&mut self, file_name: &str, now: u64) -> Option<Crash> {
        let crash = core_file(file_name, self.uid)?;
        if !self.seen.insert(file_name.to_owned()) {
            return None;
        }
        let key = format!("comm:{}", crash.comm.as_deref().unwrap_or(""));
        self.limiter.allow(&key, now).then_some(crash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn notified(units: Vec<UnitName>) -> Vec<String> {
        units.iter().map(|u| u.as_str().to_owned()).collect()
    }

    #[test]
    fn settings_default_and_parse() {
        assert_eq!(
            Settings::default(),
            Settings {
                failed_units: false,
                coredumps: false
            }
        );
        assert_eq!(Settings::parse(""), Settings::default());
        let s = Settings::parse(
            "# mine\n rebuild-failed = false\nfailed-units=true\ncoredumps = yes\nother = true\n",
        );
        assert_eq!(
            s,
            Settings {
                failed_units: true,
                coredumps: false
            }
        );
        assert!(Settings::parse("coredumps = true").coredumps);
        // The removed rebuild trigger's key is just an unknown key now.
        assert_eq!(
            Settings::parse("rebuild-failed = true"),
            Settings::default()
        );
    }

    #[test]
    fn units_already_failed_at_the_first_read_are_not_notified() {
        let mut w = FailedUnits::default();
        assert!(w.observe(&names(&["a.service", "b.service"]), 0).is_empty());
        assert_eq!(
            notified(w.observe(&names(&["a.service", "b.service", "c.service"]), 60)),
            ["c.service"]
        );
    }

    #[test]
    fn a_unit_is_notified_once_while_it_stays_failed() {
        let mut w = FailedUnits::default();
        w.observe(&[], 0);
        assert_eq!(
            notified(w.observe(&names(&["x.service"]), 60)),
            ["x.service"]
        );
        for t in 2..100 {
            assert!(w.observe(&names(&["x.service"]), t * 60).is_empty());
        }
    }

    #[test]
    fn a_flapping_unit_waits_for_its_cooldown() {
        let mut w = FailedUnits::default();
        w.observe(&[], 0);
        assert_eq!(w.observe(&names(&["x.service"]), 60).len(), 1);
        w.observe(&[], 120);
        assert!(w.observe(&names(&["x.service"]), 180).is_empty());
        w.observe(&[], 240);
        assert_eq!(w.observe(&names(&["x.service"]), 60 + COOLDOWN).len(), 1);
    }

    #[test]
    fn many_failures_at_once_are_capped() {
        let mut w = FailedUnits::default();
        w.observe(&[], 0);
        let ten: Vec<String> = (0..10).map(|i| format!("u{i}.service")).collect();
        assert_eq!(w.observe(&ten, 60).len(), MAX_PER_WINDOW);
        let more: Vec<String> = (0..20).map(|i| format!("u{i}.service")).collect();
        assert!(w.observe(&more, 120).is_empty());
        let later: Vec<String> = (0..21).map(|i| format!("u{i}.service")).collect();
        assert_eq!(notified(w.observe(&later, 60 + WINDOW)), ["u20.service"]);
    }

    #[test]
    fn invalid_unit_names_are_never_notified() {
        let mut w = FailedUnits::default();
        w.observe(&[], 0);
        let bad = names(&["-rf.service", "a;b.service", "x/../y.service", "ok.service"]);
        assert_eq!(notified(w.observe(&bad, 60)), ["ok.service"]);
    }

    #[test]
    fn crashes_are_own_uid_once_each_and_limited() {
        let boot = "0123456789abcdef0123456789abcdef";
        let file = |comm: &str, uid: u32, pid: u32| format!("core.{comm}.{uid}.{boot}.{pid}.1.zst");
        let mut w = Crashes::new(1000);
        assert_eq!(w.observe(&file("app", 0, 1), 0), None);
        let first = w.observe(&file("app", 1000, 2), 0).expect("own crash");
        assert_eq!((first.comm.as_deref(), first.pid), (Some("app"), 2));
        assert_eq!(w.observe(&file("app", 1000, 2), 1), None);
        // A crash loop: one notification per cooldown.
        for pid in 3..50 {
            assert_eq!(w.observe(&file("app", 1000, pid), u64::from(pid)), None);
        }
        assert!(w.observe(&file("app", 1000, 99), COOLDOWN + 1).is_some());
        assert!(w.observe(&file("other", 1000, 100), COOLDOWN + 2).is_some());
        assert_eq!(w.observe("core.garbage", COOLDOWN + 3), None);
        // The same file again, long after: still once.
        assert_eq!(w.observe(&file("app", 1000, 2), 10 * COOLDOWN), None);
    }
}
