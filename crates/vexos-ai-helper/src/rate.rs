//! How often each method may be called: the limits of section 3.4 of the design
//! note. Pure: the caller passes the time, so tests need no clock.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// The span the limits are counted over.
pub const WINDOW: Duration = Duration::from_secs(1);

/// The most calls of a method in one [`WINDOW`]. A method not listed has no
/// limit.
pub const LIMITS: [(&str, usize); 5] = [
    ("Send", 2),
    ("Decide", 5),
    ("Edited", 10),
    ("Shown", 10),
    ("DraftChoice", 2),
];

/// Counts accepted calls over a sliding [`WINDOW`]. A refused call is not
/// counted, so a flood cannot keep the next second closed.
#[derive(Debug, Default)]
pub struct Limiter {
    accepted: HashMap<&'static str, VecDeque<Instant>>,
}

impl Limiter {
    /// Whether a call of `method` at `now` is within its limit; if so, it is
    /// counted.
    pub fn allow(&mut self, method: &str, now: Instant) -> bool {
        let Some((name, limit)) = LIMITS.iter().find(|(name, _)| *name == method) else {
            return true;
        };
        let seen = self.accepted.entry(name).or_default();
        while seen
            .front()
            .is_some_and(|&then| now.saturating_duration_since(then) >= WINDOW)
        {
            seen.pop_front();
        }
        if seen.len() >= *limit {
            return false;
        }
        seen.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64, base: Instant) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn the_limits_are_the_ones_in_the_design_note() {
        assert_eq!(
            LIMITS,
            [
                ("Send", 2),
                ("Decide", 5),
                ("Edited", 10),
                ("Shown", 10),
                ("DraftChoice", 2)
            ]
        );
        assert_eq!(WINDOW, Duration::from_secs(1));
    }

    #[test]
    fn every_method_stops_at_its_own_limit_and_not_before() {
        let base = Instant::now();
        for (method, limit) in LIMITS {
            let mut limiter = Limiter::default();
            for n in 0..limit {
                assert!(limiter.allow(method, base), "{method} call {n}");
            }
            assert!(!limiter.allow(method, base), "{method} over its limit");
            assert!(!limiter.allow(method, at(999, base)), "{method} at 999 ms");
        }
    }

    #[test]
    fn a_method_may_be_called_again_a_second_after_the_first_call() {
        let base = Instant::now();
        let mut limiter = Limiter::default();
        assert!(limiter.allow("Send", base));
        assert!(limiter.allow("Send", at(500, base)));
        assert!(!limiter.allow("Send", at(999, base)));
        // The first call is a full second old: one place is free, not two.
        assert!(limiter.allow("Send", at(1000, base)));
        assert!(!limiter.allow("Send", at(1000, base)));
        assert!(limiter.allow("Send", at(1500, base)));
    }

    #[test]
    fn the_methods_are_counted_apart() {
        let base = Instant::now();
        let mut limiter = Limiter::default();
        assert!(limiter.allow("Send", base));
        assert!(limiter.allow("Send", base));
        assert!(!limiter.allow("Send", base));
        assert!(limiter.allow("DraftChoice", base));
        assert!(limiter.allow("Edited", base));
        assert!(limiter.allow("Decide", base));
    }

    #[test]
    fn a_refused_call_does_not_keep_the_window_closed() {
        let base = Instant::now();
        let mut limiter = Limiter::default();
        assert!(limiter.allow("Send", base));
        assert!(limiter.allow("Send", base));
        for ms in 1..1000 {
            assert!(!limiter.allow("Send", at(ms, base)));
        }
        assert!(limiter.allow("Send", at(1000, base)));
    }

    #[test]
    fn the_other_methods_have_no_limit() {
        let base = Instant::now();
        let mut limiter = Limiter::default();
        for method in ["Open", "Close", "Cancel", "CheckUsage", "DraftDiscard"] {
            for _ in 0..1000 {
                assert!(limiter.allow(method, base), "{method}");
            }
        }
        // An unknown name has no limit here either: the gate refuses it first.
        assert!(limiter.allow("Nope", base));
    }
}
