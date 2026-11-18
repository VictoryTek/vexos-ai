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

/// What to do about the list after its scroll value changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// At the end: follow from here on.
    Resume,
    /// Off the end because the user scrolled: stop following, leave it.
    Stop,
    /// Following, and something other than the user moved the list off the
    /// end (a focus change, a layout clamp): put it back at the end.
    Restore,
    /// Not following: the user is reading; leave it.
    Leave,
}

/// The rule for a changed scroll value. `user_active` is whether the user just
/// did something to the list (wheel, click, key, touch). Without it a move off
/// the end is not theirs, so it cannot end following.
pub fn after_move(following: bool, user_active: bool, value: f64, upper: f64, page: f64) -> Moved {
    if at_end(value, upper, page) {
        Moved::Resume
    } else if !following {
        Moved::Leave
    } else if user_active {
        Moved::Stop
    } else {
        Moved::Restore
    }
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
    fn a_jump_nobody_asked_for_is_put_back_not_obeyed() {
        // At the end of 3000 px, and something moved the list: not the user.
        assert_eq!(after_move(true, false, 0.0, 3000.0, 400.0), Moved::Restore);
        assert_eq!(
            after_move(true, false, 1200.0, 3000.0, 400.0),
            Moved::Restore
        );
    }

    #[test]
    fn the_users_own_scroll_up_stops_following() {
        assert_eq!(after_move(true, true, 1200.0, 3000.0, 400.0), Moved::Stop);
        assert_eq!(after_move(true, true, 0.0, 3000.0, 400.0), Moved::Stop);
    }

    #[test]
    fn a_reader_who_stopped_following_is_left_alone_until_the_end() {
        assert_eq!(after_move(false, true, 1200.0, 3000.0, 400.0), Moved::Leave);
        assert_eq!(
            after_move(false, false, 1200.0, 3000.0, 400.0),
            Moved::Leave
        );
        assert_eq!(
            after_move(false, true, 2600.0, 3000.0, 400.0),
            Moved::Resume
        );
        assert_eq!(
            after_move(false, false, 2590.0, 3000.0, 400.0),
            Moved::Resume
        );
    }

    #[test]
    fn at_the_end_following_goes_on_whoever_moved_it() {
        for active in [true, false] {
            assert_eq!(
                after_move(true, active, 2600.0, 3000.0, 400.0),
                Moved::Resume
            );
            assert_eq!(after_move(true, active, 0.0, 300.0, 400.0), Moved::Resume);
        }
    }

    #[test]
    fn bad_numbers_never_panic() {
        for v in [f64::NAN, f64::INFINITY, -1.0] {
            let _ = at_end(v, v, v);
            let _ = end_value(v, v);
            for following in [true, false] {
                for active in [true, false] {
                    let _ = after_move(following, active, v, v, v);
                }
            }
        }
    }
}
