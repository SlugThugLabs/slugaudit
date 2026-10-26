//! Process timestamps and cooperative operation deadlines.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime};

static LAST_UNIX_TIME: AtomicI64 = AtomicI64::new(0);

/// The current Unix timestamp in seconds. A clock before the epoch (or past
/// the `i64` range) is a documented, deliberate recovery: it yields `0`
/// (respectively `i64::MAX`), logged with the source error, and the monotonic
/// guard below keeps stored timestamps from moving backwards.
pub(crate) fn now_unix() -> i64 {
    let observed = crate::clock::unix_seconds(SystemTime::now());
    let mut previous = LAST_UNIX_TIME.load(Ordering::Relaxed);
    loop {
        let next = observed.max(previous);
        match LAST_UNIX_TIME.compare_exchange_weak(
            previous,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

/// Returns a wall-clock timestamp that is at least the last persisted value.
/// SQLite row IDs remain the canonical ordering key; this only prevents the
/// human-readable timestamps from moving backwards across restarts.
pub(crate) fn at_least_timestamp(observed: i64, persisted: Option<i64>) -> i64 {
    observed.max(persisted.unwrap_or(0))
}

/// A cooperative wall-clock deadline for one sync operation.
///
/// Created once at the entry of an operation that may iterate many times
/// (a publish spanning CAS retries, a barrier sync spanning reconcile
/// iterations) so the budget covers the whole operation rather than
/// resetting per phase — otherwise a pathological repo could restart the
/// clock on every retry and stall a tool call indefinitely.
///
/// `exceeded` is checked at cooperative points inside hot loops (per
/// discovered file, per dirty path, per barrier iteration). It never
/// interrupts anything; it only reports that the budget has been spent, so
/// the loop can fail closed with a `TimeBudgetExceeded` error instead of
/// looping forever.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    started: Instant,
    budget: Duration,
}

impl Deadline {
    pub(crate) fn after(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }

    /// Returns the elapsed time if the budget is exhausted, else `None`.
    /// Tests inject `Duration::from_nanos(1)` so any positive elapsed time
    /// trips the deadline deterministically without waiting out the
    /// production-sized budget.
    pub(crate) fn exceeded(&self) -> Option<Duration> {
        let elapsed = self.started.elapsed();
        (elapsed >= self.budget).then_some(elapsed)
    }

    /// A deadline for one unit of work inside this operation: `cap` from now,
    /// or this deadline itself when it would end sooner (or at the same
    /// moment). The returned [`BudgetBound`] says which of the two the unit
    /// runs under, so a caller that sees the unit cut at its deadline knows,
    /// by type rather than by comparing clocks afterwards, whether the unit's
    /// own allowance ran out (the operation goes on) or the operation's did
    /// (the operation is over).
    pub(crate) fn capped(&self, cap: Duration) -> (Self, BudgetBound) {
        let remaining = self.budget.saturating_sub(self.started.elapsed());
        if cap < remaining {
            (Self::after(cap), BudgetBound::Unit)
        } else {
            (*self, BudgetBound::Operation)
        }
    }

    /// Whole milliseconds left in the budget, rounded down, for an API that
    /// takes a millisecond timeout (the language pack's parse timeout).
    /// `None` once less than one millisecond remains: a sub-millisecond
    /// remainder counts as spent, so a caller never hands a zero timeout
    /// (which the pack rejects, and which would otherwise read as "no
    /// limit") to a parser. Saturates at `u64::MAX` milliseconds.
    pub(crate) fn remaining_millis(&self) -> Option<NonZeroU64> {
        let remaining = self.budget.checked_sub(self.started.elapsed())?;
        let millis = remaining
            .as_secs()
            .saturating_mul(1_000)
            .saturating_add(u64::from(remaining.subsec_millis()));
        NonZeroU64::new(millis)
    }
}

/// Which budget a [`Deadline::capped`] unit of work runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetBound {
    /// The unit's own cap is the sooner one: running out of it ends the unit
    /// only, and the operation still has time.
    Unit,
    /// The operation's deadline is the sooner one (or ties): running out of
    /// it ends the operation.
    Operation,
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn a_zero_budget_is_exceeded_immediately() {
        let deadline = Deadline::after(Duration::ZERO);
        assert!(deadline.exceeded().is_some());
    }

    #[test]
    fn a_generous_budget_is_not_exceeded_immediately() {
        let deadline = Deadline::after(Duration::from_secs(60));
        assert!(deadline.exceeded().is_none());
    }

    #[test]
    fn a_spent_budget_has_no_milliseconds_remaining() {
        let deadline = Deadline::after(Duration::ZERO);

        assert_eq!(deadline.remaining_millis(), None);
    }

    #[test]
    fn a_sub_millisecond_remainder_counts_as_spent() {
        let deadline = Deadline::after(Duration::from_micros(999));

        assert_eq!(deadline.remaining_millis(), None);
    }

    #[test]
    fn a_live_budget_reports_its_whole_remaining_milliseconds() {
        let deadline = Deadline::after(Duration::from_secs(60));

        let remaining = deadline.remaining_millis().expect("time is left");

        assert!(
            remaining.get() > 59_000 && remaining.get() <= 60_000,
            "{remaining}"
        );
    }

    #[test]
    fn a_cap_below_the_remaining_budget_is_a_unit_bound() {
        let operation = Deadline::after(Duration::from_secs(60));

        let (unit, bound) = operation.capped(Duration::from_secs(5));

        assert_eq!(bound, BudgetBound::Unit);
        let remaining = unit.remaining_millis().expect("time is left").get();
        assert!((4_000..=5_000).contains(&remaining), "{remaining}");
    }

    #[test]
    fn a_cap_above_the_remaining_budget_is_an_operation_bound() {
        let operation = Deadline::after(Duration::from_secs(5));

        let (unit, bound) = operation.capped(Duration::from_secs(60));

        assert_eq!(bound, BudgetBound::Operation);
        let remaining = unit.remaining_millis().expect("time is left").get();
        assert!(remaining <= 5_000, "{remaining}");
    }

    #[test]
    fn a_spent_operation_is_an_operation_bound_whatever_the_cap() {
        let operation = Deadline::after(Duration::ZERO);

        let (unit, bound) = operation.capped(Duration::from_secs(5));

        assert_eq!(bound, BudgetBound::Operation);
        assert!(unit.exceeded().is_some());
    }

    #[test]
    fn a_zero_cap_is_a_spent_unit_bound() {
        let operation = Deadline::after(Duration::from_secs(60));

        let (unit, bound) = operation.capped(Duration::ZERO);

        assert_eq!(bound, BudgetBound::Unit);
        assert!(unit.exceeded().is_some());
    }
}
