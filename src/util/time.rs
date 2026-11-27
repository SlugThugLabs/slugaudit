//! Process timestamps and cooperative operation deadlines.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime};

static LAST_UNIX_TIME: AtomicI64 = AtomicI64::new(0);

/// The current Unix timestamp in seconds. The monotonic guard below keeps
/// stored timestamps from moving backwards.
///
/// # Errors
///
/// Returns [`crate::clock::ClockError`] when the system clock is before the
/// epoch or past the `i64` range.
pub(crate) fn now_unix() -> Result<i64, crate::clock::ClockError> {
    let observed = crate::clock::unix_seconds(SystemTime::now())?;
    let mut previous = LAST_UNIX_TIME.load(Ordering::Relaxed);
    loop {
        let next = observed.max(previous);
        match LAST_UNIX_TIME.compare_exchange_weak(
            previous,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(next),
            Err(actual) => previous = actual,
        }
    }
}

/// Returns a wall-clock timestamp that is at least the last persisted value.
/// SQLite row IDs remain the canonical ordering key; this only prevents the
/// human-readable timestamps from moving backwards across restarts.
pub(crate) fn at_least_timestamp(observed: i64, persisted: Option<i64>) -> i64 {
    match persisted {
        Some(persisted) => observed.max(persisted),
        None => observed,
    }
}

/// A cooperative wall-clock deadline for one sync operation.
///
/// Created once at the entry of an operation that may iterate many times
/// (a publish spanning retries) so the budget covers the whole operation
/// rather than resetting per phase — otherwise a pathological repo could
/// restart the clock on every retry and stall a tool call indefinitely.
///
/// `exceeded` is checked at cooperative points inside hot loops (per
/// discovered file, per sampled file, per verified row). It never
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
        // A budget already spent has nothing remaining, so no cap is sooner.
        match self.budget.checked_sub(self.started.elapsed()) {
            Some(remaining) if cap < remaining => (Self::after(cap), BudgetBound::Unit),
            _ => (*self, BudgetBound::Operation),
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
        // KEEP saturating: a timeout past u64::MAX milliseconds never fires either way, so the clamp changes no behavior
        let millis = remaining
            .as_secs()
            .saturating_mul(1_000)
            // KEEP saturating: same clamp as the multiplication above
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
#[path = "time_tests.rs"]
mod deadline_tests;
