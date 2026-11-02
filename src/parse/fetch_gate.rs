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
    use std::sync::Mutex;
    use std::time::Instant;

    static LAST_FAILURE: Mutex<Option<Instant>> = Mutex::new(None);

    pub(super) fn now() -> Instant {
        Instant::now()
    }

    /// Runs `change` on the time of the last fetch failure.
    pub(super) fn with_last_failure<T>(
        change: impl FnOnce(&mut Option<Instant>) -> T,
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
        static LAST_FAILURE: Mutex<Option<Instant>> = const { Mutex::new(None) };
        static SKEW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static CACHED: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn now() -> Instant {
        Instant::now() + SKEW.with(Cell::get)
    }

    /// Runs `change` on this thread's time of the last fetch failure.
    pub(super) fn with_last_failure<T>(
        change: impl FnOnce(&mut Option<Instant>) -> T,
    ) -> Result<T, LockPoisoned> {
        LAST_FAILURE.with(|last| Ok(change(&mut *crate::util::lock(last, "grammar_fetch_gate")?)))
    }

    pub(super) fn grammar_cached(_language: &str) -> bool {
        CACHED.with(Cell::get)
    }

    pub(super) fn advance(by: Duration) {
        SKEW.with(|skew| skew.set(skew.get() + by));
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
        last.is_some_and(|at| now.saturating_duration_since(at) < GRAMMAR_FETCH_COOLDOWN)
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

    /// Moves this thread's gate clock past the cool-down.
    pub(crate) fn elapse_cooldown() {
        state::advance(GRAMMAR_FETCH_COOLDOWN + std::time::Duration::from_secs(1));
    }

    /// Makes `blocks_fetch` treat every grammar as already cached (or not).
    pub(crate) fn set_grammar_cached(cached: bool) {
        state::set_cached(cached);
    }
}
