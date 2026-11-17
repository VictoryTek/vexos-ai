//! When the chat list follows new text. Plain numbers (an adjustment's value,
//! upper bound and page size, in pixels), so the rule is tested headless.
//!
//! The list follows its end while the user is at the end, stops when they
//! scroll up to read, and follows again when they come back down or send a
//! message. The window scrolls only after the new text has been laid out, so a
//! growing reply is followed steadily instead of jumping.

/// How far above the very end still counts as "at the end".
pub const SLACK: f64 = 24.0;

/// The user is at the end of the list: the value that shows the last pixel, give
/// or take `SLACK`.
pub fn at_end(value: f64, upper: f64, page: f64) -> bool {
    value + page >= upper - SLACK
}

/// The value that shows the end of the list. Never negative: a list shorter
/// than its window stays at the top.
pub fn end_value(upper: f64, page: f64) -> f64 {
    (upper - page).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_the_end_exactly_or_within_the_slack() {
        assert!(at_end(600.0, 1000.0, 400.0));
        assert!(at_end(580.0, 1000.0, 400.0));
        assert!(!at_end(570.0, 1000.0, 400.0));
    }

    #[test]
    fn scrolled_up_to_read_is_not_the_end() {
        assert!(!at_end(0.0, 2000.0, 400.0));
        assert!(!at_end(900.0, 2000.0, 400.0));
    }

    #[test]
    fn a_list_shorter_than_its_window_is_always_at_the_end() {
        assert!(at_end(0.0, 300.0, 400.0));
        assert_eq!(end_value(300.0, 400.0), 0.0);
    }

    #[test]
    fn the_end_value_shows_the_last_pixel() {
        assert_eq!(end_value(1000.0, 400.0), 600.0);
        assert!(at_end(end_value(1234.5, 400.0), 1234.5, 400.0));
    }

    #[test]
    fn growth_alone_does_not_end_following() {
        // The value stays while the list grows below it: only a change of the
        // value (the user) is judged, and judged against the new size.
        let value = end_value(1000.0, 400.0);
        assert!(at_end(value, 1000.0, 400.0));
        assert!(!at_end(value, 1500.0, 400.0));
    }

    #[test]
    fn bad_numbers_never_panic() {
        for v in [f64::NAN, f64::INFINITY, -1.0] {
            let _ = at_end(v, v, v);
            let _ = end_value(v, v);
        }
    }
}
