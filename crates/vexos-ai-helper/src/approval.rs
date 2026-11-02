//! The rules for answering an approval request, with no D-Bus and no session in
//! them: a `Decide` is allowed only for the current generation, for a request
//! that is pending right now, that the Shell reported drawing, and (to approve)
//! only 400 ms after that report. They guard against bugs and click-through, not
//! against a hostile caller that is already let in (design note, 3A.4).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::gate::Choice;

/// The least time between `Shown` and an `approve-once`. `deny-once` has none.
pub const MIN_AGE: Duration = Duration::from_millis(400);

/// Why a `Decide` or `Shown` was refused. The caller is always told the same
/// fixed error; this is for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The call names another session than this one.
    StaleGeneration,
    /// No such request, or it has been answered already.
    NotPending,
    /// The Shell never reported drawing this card.
    NotShown,
    /// `approve-once` came less than [`MIN_AGE`] after `Shown`.
    TooEarly,
}

pub struct Approvals {
    generation: u64,
    /// When each card was first reported shown.
    shown: HashMap<u64, Instant>,
}

impl Approvals {
    pub fn new(generation: u64) -> Approvals {
        Approvals {
            generation,
            shown: HashMap::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether this `Shown` is the first for a pending card of this session, and
    /// so exempt from the rate limit. A repeat, a stale generation and a card
    /// that is not pending are not.
    pub fn first_shown(&self, generation: u64, request: u64, pending: bool) -> bool {
        generation == self.generation && pending && !self.shown.contains_key(&request)
    }

    /// The Shell drew the card. Only the first report counts: a repeat does not
    /// restart the clock.
    pub fn shown(
        &mut self,
        generation: u64,
        request: u64,
        pending: bool,
        now: Instant,
    ) -> Result<(), Refused> {
        if generation != self.generation {
            return Err(Refused::StaleGeneration);
        }
        if !pending {
            return Err(Refused::NotPending);
        }
        self.shown.entry(request).or_insert(now);
        Ok(())
    }

    /// Whether `choice` may be passed on for `request` now.
    pub fn decide(
        &self,
        generation: u64,
        request: u64,
        choice: Choice,
        pending: bool,
        now: Instant,
    ) -> Result<(), Refused> {
        if generation != self.generation {
            return Err(Refused::StaleGeneration);
        }
        if !pending {
            return Err(Refused::NotPending);
        }
        let shown = self.shown.get(&request).ok_or(Refused::NotShown)?;
        if choice == Choice::ApproveOnce && now.saturating_duration_since(*shown) < MIN_AGE {
            return Err(Refused::TooEarly);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEN: u64 = 7;

    fn at(ms: u64, base: Instant) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn shown_at(base: Instant) -> Approvals {
        let mut a = Approvals::new(GEN);
        a.shown(GEN, 1, true, base).unwrap();
        a
    }

    #[test]
    fn the_minimum_age_is_400_ms() {
        assert_eq!(MIN_AGE, Duration::from_millis(400));
    }

    #[test]
    fn approve_waits_exactly_the_minimum_age() {
        let base = Instant::now();
        let a = shown_at(base);
        for ms in [0, 1, 399] {
            assert_eq!(
                a.decide(GEN, 1, Choice::ApproveOnce, true, at(ms, base)),
                Err(Refused::TooEarly),
                "{ms} ms"
            );
        }
        for ms in [400, 401, 10_000] {
            assert_eq!(
                a.decide(GEN, 1, Choice::ApproveOnce, true, at(ms, base)),
                Ok(()),
                "{ms} ms"
            );
        }
    }

    #[test]
    fn deny_has_no_delay() {
        let base = Instant::now();
        let a = shown_at(base);
        assert_eq!(a.decide(GEN, 1, Choice::DenyOnce, true, base), Ok(()));
    }

    #[test]
    fn a_clock_that_runs_backwards_does_not_approve() {
        let base = Instant::now() + Duration::from_secs(10);
        let a = shown_at(base);
        let earlier = base - Duration::from_secs(5);
        assert_eq!(
            a.decide(GEN, 1, Choice::ApproveOnce, true, earlier),
            Err(Refused::TooEarly)
        );
    }

    #[test]
    fn a_card_never_shown_cannot_be_decided_either_way() {
        let base = Instant::now();
        let a = Approvals::new(GEN);
        for choice in [Choice::ApproveOnce, Choice::DenyOnce] {
            assert_eq!(
                a.decide(GEN, 1, choice, true, at(5_000, base)),
                Err(Refused::NotShown)
            );
        }
        // Another card being shown does not count.
        let mut a = Approvals::new(GEN);
        a.shown(GEN, 2, true, base).unwrap();
        assert_eq!(
            a.decide(GEN, 1, Choice::DenyOnce, true, at(5_000, base)),
            Err(Refused::NotShown)
        );
    }

    #[test]
    fn another_generation_is_refused_for_shown_and_decide() {
        let base = Instant::now();
        for wrong in [GEN - 1, GEN + 1, 0, u64::MAX] {
            let mut a = Approvals::new(GEN);
            assert_eq!(a.shown(wrong, 1, true, base), Err(Refused::StaleGeneration));
            let a = shown_at(base);
            for choice in [Choice::ApproveOnce, Choice::DenyOnce] {
                assert_eq!(
                    a.decide(wrong, 1, choice, true, at(5_000, base)),
                    Err(Refused::StaleGeneration)
                );
            }
        }
    }

    #[test]
    fn a_request_that_is_not_pending_is_refused_for_shown_and_decide() {
        let base = Instant::now();
        let mut a = Approvals::new(GEN);
        assert_eq!(a.shown(GEN, 1, false, base), Err(Refused::NotPending));
        // The card was shown, then answered: it is no longer pending.
        let a = shown_at(base);
        for choice in [Choice::ApproveOnce, Choice::DenyOnce] {
            assert_eq!(
                a.decide(GEN, 1, choice, false, at(5_000, base)),
                Err(Refused::NotPending)
            );
        }
    }

    #[test]
    fn a_refused_shown_is_not_remembered() {
        let base = Instant::now();
        let mut a = Approvals::new(GEN);
        let _ = a.shown(GEN, 1, false, base);
        let _ = a.shown(GEN + 1, 1, true, base);
        assert_eq!(
            a.decide(GEN, 1, Choice::DenyOnce, true, at(5_000, base)),
            Err(Refused::NotShown)
        );
    }

    #[test]
    fn a_repeated_shown_does_not_restart_the_clock() {
        let base = Instant::now();
        let mut a = shown_at(base);
        a.shown(GEN, 1, true, at(300, base)).unwrap();
        assert_eq!(
            a.decide(GEN, 1, Choice::ApproveOnce, true, at(400, base)),
            Ok(())
        );
    }

    #[test]
    fn the_cards_are_timed_apart() {
        let base = Instant::now();
        let mut a = shown_at(base);
        a.shown(GEN, 2, true, at(300, base)).unwrap();
        assert_eq!(
            a.decide(GEN, 1, Choice::ApproveOnce, true, at(400, base)),
            Ok(())
        );
        assert_eq!(
            a.decide(GEN, 2, Choice::ApproveOnce, true, at(400, base)),
            Err(Refused::TooEarly)
        );
    }

    #[test]
    fn only_the_first_shown_of_a_pending_card_of_this_session_is_exempt() {
        let base = Instant::now();
        let mut a = Approvals::new(GEN);
        assert!(a.first_shown(GEN, 1, true));
        assert!(!a.first_shown(GEN + 1, 1, true), "stale generation");
        assert!(!a.first_shown(GEN, 1, false), "not pending");
        a.shown(GEN, 1, true, base).unwrap();
        assert!(!a.first_shown(GEN, 1, true), "a repeat");
        assert!(a.first_shown(GEN, 2, true), "another card");
    }

    #[test]
    fn the_generation_is_the_one_it_was_made_with() {
        assert_eq!(Approvals::new(42).generation(), 42);
    }
}
