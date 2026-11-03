//! Process-wide cool-down for grammar fetches.
//!
//! The language pack downloads a grammar bundle the first time a language is
//! parsed, and a failed attempt can block for the pack's whole HTTP timeout
//! (60 s, a constant inside the pack: it exposes no setting for it). Without
//! a gate every file, and every sync after it, would pay that cost again on a
//! host that is offline. After one fetch failure the gate closes for
//! [`GRAMMAR_FETCH_COOLDOWN`]: grammars that are not already on this host are
//! not requested, and stored `LoadFailed` rows are not retry candidates. A
//! grammar that is already cached still loads normally. The state is per
//! process, so a server restart retries immediately.

use crate::util::LockPoisoned;
use std::time::Duration;

/// How long the gate stays closed after a fetch failure. Long enough that an
/// offline host pays the fetch timeout once per session rather than once per
/// tool call, short enough that regaining the network is noticed within a
/// working session without a restart.
pub(crate) const GRAMMAR_FETCH_COOLDOWN: Duration = Duration::from_mins(10);

#[cfg(not(test))]
mod state {
    use crate::util::LockPoisoned;
    use std::sync::{LazyLock, Mutex};
    use std::time::{Duration, Instant};

    static LAST_FAILURE: Mutex<Option<Duration>> = Mutex::new(None);
    /// The instant the gate's timeline starts at: its first use.
    static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);

    /// The gate's clock: the time elapsed since [`ORIGIN`].
    pub(super) fn now() -> Duration {
        ORIGIN.elapsed()
    }

    /// Runs `change` on the time of the last fetch failure.
    pub(super) fn with_last_failure<T>(
        change: impl FnOnce(&mut Option<Duration>) -> T,
    ) -> Result<T, LockPoisoned> {
        Ok(change(&mut *crate::util::lock(
            &LAST_FAILURE,
            "grammar_fetch_gate",
        )?))
    }

    pub(super) fn grammar_cached(language: &str) -> bool {
        tree_sitter_language_pack::has_parser(language)
    }
}

/// Tests get a gate, a clock and a "what is cached" answer of their own per
/// thread, so parallel tests cannot observe each other and nothing depends on
/// the host's grammar cache.
#[cfg(test)]
mod state {
    use crate::util::LockPoisoned;
    use std::cell::Cell;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    thread_local! {
        static LAST_FAILURE: Mutex<Option<Duration>> = const { Mutex::new(None) };
        static SKEW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static CACHED: Cell<bool> = const { Cell::new(false) };
        /// The instant this thread's gate timeline starts at.
        static ORIGIN: Instant = Instant::now();
    }

    /// The gate's clock: the time elapsed since this thread's origin, plus
    /// the skew tests added. A skew that would exceed `Duration::MAX`
    /// saturates there: that much elapsed time is past every cool-down,
    /// which is what saturation means for a clock.
    pub(super) fn now() -> Duration {
        // KEEP saturating: a clock reading past Duration::MAX is past every cool-down, so the clamped reading gives the same answer
        ORIGIN
            .with(Instant::elapsed)
            .saturating_add(SKEW.with(Cell::get))
    }

    /// Runs `change` on this thread's time of the last fetch failure.
    pub(super) fn with_last_failure<T>(
        change: impl FnOnce(&mut Option<Duration>) -> T,
    ) -> Result<T, LockPoisoned> {
        LAST_FAILURE.with(|last| Ok(change(&mut *crate::util::lock(last, "grammar_fetch_gate")?)))
    }

    pub(super) fn grammar_cached(_language: &str) -> bool {
        CACHED.with(Cell::get)
    }

    pub(super) fn advance(by: Duration) {
        // KEEP saturating: a skew past Duration::MAX is past every cool-down, so the clamped skew gives the same answer
        SKEW.with(|skew| skew.set(skew.get().saturating_add(by)));
    }

    pub(super) fn set_cached(cached: bool) {
        CACHED.with(|flag| flag.set(cached));
    }
}

/// Records that a grammar fetch just failed, closing the gate.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the gate's state was poisoned.
pub(crate) fn record_fetch_failure() -> Result<(), LockPoisoned> {
    let now = state::now();
    state::with_last_failure(|last| *last = Some(now))
}

fn cooling_down() -> Result<bool, LockPoisoned> {
    let now = state::now();
    state::with_last_failure(|last| {
        // A failure stamped after `now` has not aged at all: still cooling down.
        last.is_some_and(|at| {
            now.checked_sub(at)
                .is_none_or(|elapsed| elapsed < GRAMMAR_FETCH_COOLDOWN)
        })
    })
}

/// True when a stored `LoadFailed` row should be re-parsed now: no fetch has
/// failed in this process, or the cool-down has elapsed. Every sync asks
/// this, so during the cool-down an unchanged
/// `LoadFailed` file is skipped like any other and no revision is published.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the gate's state was poisoned.
pub(crate) fn load_failed_retry_due() -> Result<bool, LockPoisoned> {
    Ok(!cooling_down()?)
}

/// True when extraction for `language` must not call the pack: the gate is
/// closed and the grammar is not already on this host (the pack's
/// `has_parser`), so the call would attempt a download.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the gate's state was poisoned.
pub(crate) fn blocks_fetch(language: &str) -> Result<bool, LockPoisoned> {
    Ok(cooling_down()? && !state::grammar_cached(language))
}

/// Test controls for the gate's clock and grammar cache.
#[cfg(test)]
pub(crate) mod test_controls {
    use super::{GRAMMAR_FETCH_COOLDOWN, state};

    /// One second past the cool-down; a compile-time sum, so it cannot
    /// overflow at run time.
    const PAST_COOLDOWN: std::time::Duration =
        std::time::Duration::from_secs(GRAMMAR_FETCH_COOLDOWN.as_secs() + 1);

    /// Moves this thread's gate clock past the cool-down.
    pub(crate) fn elapse_cooldown() {
        state::advance(PAST_COOLDOWN);
    }

    /// Makes `blocks_fetch` treat every grammar as already cached (or not).
    pub(crate) fn set_grammar_cached(cached: bool) {
        state::set_cached(cached);
    }
}
