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

use std::time::Duration;

/// How long the gate stays closed after a fetch failure. Long enough that an
/// offline host pays the fetch timeout once per session rather than once per
/// tool call, short enough that regaining the network is noticed within a
/// working session without a restart.
pub(crate) const GRAMMAR_FETCH_COOLDOWN: Duration = Duration::from_secs(10 * 60);

#[cfg(not(test))]
mod state {
    use std::sync::Mutex;
    use std::time::Instant;

    static LAST_FAILURE: Mutex<Option<Instant>> = Mutex::new(None);

    pub(super) fn now() -> Instant {
        Instant::now()
    }

    pub(super) fn last_failure() -> Option<Instant> {
        *crate::util::lock_or_recover(&LAST_FAILURE, "grammar_fetch_gate")
    }

    pub(super) fn set_last_failure(at: Instant) {
        *crate::util::lock_or_recover(&LAST_FAILURE, "grammar_fetch_gate") = Some(at);
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
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    thread_local! {
        static LAST_FAILURE: Cell<Option<Instant>> = const { Cell::new(None) };
        static SKEW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static CACHED: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn now() -> Instant {
        Instant::now() + SKEW.with(Cell::get)
    }

    pub(super) fn last_failure() -> Option<Instant> {
        LAST_FAILURE.with(Cell::get)
    }

    pub(super) fn set_last_failure(at: Instant) {
        LAST_FAILURE.with(|last| last.set(Some(at)));
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
pub(crate) fn record_fetch_failure() {
    state::set_last_failure(state::now());
}

fn cooling_down() -> bool {
    state::last_failure()
        .is_some_and(|at| state::now().saturating_duration_since(at) < GRAMMAR_FETCH_COOLDOWN)
}

/// True when a stored `LoadFailed` row should be re-parsed now: no fetch has
/// failed in this process, or the cool-down has elapsed. The sweep, reconcile
/// and publish all ask this, so during the cool-down an unchanged
/// `LoadFailed` file is skipped like any other and no revision is published.
pub(crate) fn load_failed_retry_due() -> bool {
    !cooling_down()
}

/// True when extraction for `language` must not call the pack: the gate is
/// closed and the grammar is not already on this host (the pack's
/// `has_parser`), so the call would attempt a download.
pub(crate) fn blocks_fetch(language: &str) -> bool {
    cooling_down() && !state::grammar_cached(language)
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
