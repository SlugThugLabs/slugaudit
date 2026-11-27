// slugaudit-line-exception: approved-by=split-judge; reason=process-wide single-flight grammar fetch lock, cool-down gate, and thread-isolated test controls
//! Process-wide single-flight lock and cool-down for remote grammar fetches.
//!
//! The language pack downloads a grammar bundle the first time an unbundled
//! language is parsed, and a failed attempt can block for the pack's HTTP
//! timeout (60 s). After one fetch failure the gate closes for
//! [`GRAMMAR_FETCH_COOLDOWN`]: grammars that are not already on this host are
//! not requested, and stored `LoadFailed` rows are not retry candidates. A
//! grammar that is already cached (including statically bundled Rust) still
//! loads without acquiring the fetch lock.

use crate::util::LockPoisoned;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long the gate stays closed after a fetch failure.
pub(crate) const GRAMMAR_FETCH_COOLDOWN: Duration = Duration::from_mins(10);

/// Outcome of attempting to enter a grammar-fetch slot for a language.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FetchSlot<T> {
    /// The fetch gate is cooling down and the grammar is not cached.
    Blocked,
    /// `work` ran (either unserialized because the grammar was already cached,
    /// or serialized under the single-flight fetch lock).
    Ran(T),
}

enum SlotEligibility {
    Cached,
    Blocked,
    NeedsFetch,
}

/// Thread-safe state for a grammar-fetch cool-down gate and its single-flight
/// fetch lock.
pub(crate) struct GateState {
    fetch_lock: Mutex<()>,
    last_failure: Mutex<Option<Duration>>,
    origin: Instant,
    #[cfg(test)]
    hooks: test_controls::TestHooks,
}

impl GateState {
    /// Creates a gate backed by [`tree_sitter_language_pack::has_parser`].
    pub(super) fn new() -> Self {
        Self {
            fetch_lock: Mutex::new(()),
            last_failure: Mutex::new(None),
            origin: Instant::now(),
            #[cfg(test)]
            hooks: test_controls::TestHooks::host_backed(),
        }
    }

    #[cfg(test)]
    pub(super) fn new_isolated() -> Self {
        Self {
            hooks: test_controls::TestHooks::isolated_uncached(),
            ..Self::new()
        }
    }

    pub(super) fn now(&self) -> Duration {
        let elapsed = self.origin.elapsed();
        #[cfg(test)]
        // KEEP saturating: a clock reading past Duration::MAX is past every cool-down, so the clamped reading gives the same answer
        let elapsed = elapsed.saturating_add(self.hooks.skew());
        elapsed
    }

    #[cfg(test)]
    pub(super) fn grammar_cached(&self, language: &str) -> bool {
        self.hooks.grammar_cached(language)
    }

    pub(super) fn with_last_failure<T>(
        &self,
        change: impl FnOnce(&mut Option<Duration>) -> T,
    ) -> Result<T, LockPoisoned> {
        Ok(change(&mut *crate::util::lock(
            &self.last_failure,
            "grammar_fetch_gate",
        )?))
    }

    pub(super) fn record_fetch_failure(&self) -> Result<(), LockPoisoned> {
        self.with_last_failure(|last| {
            let now = self.now();
            *last = Some(last.map_or(now, |prev| prev.max(now)));
        })
    }

    pub(super) fn cooling_down(&self) -> Result<bool, LockPoisoned> {
        self.with_last_failure(|last| {
            let now = self.now();
            last.is_some_and(|at| {
                now.checked_sub(at)
                    .is_none_or(|elapsed| elapsed < GRAMMAR_FETCH_COOLDOWN)
            })
        })
    }

    fn check_eligibility(&self, language: &str) -> Result<SlotEligibility, LockPoisoned> {
        #[cfg(test)]
        let cached = self.grammar_cached(language);
        #[cfg(not(test))]
        let cached = tree_sitter_language_pack::has_parser(language);
        if cached {
            Ok(SlotEligibility::Cached)
        } else if self.cooling_down()? {
            Ok(SlotEligibility::Blocked)
        } else {
            Ok(SlotEligibility::NeedsFetch)
        }
    }

    pub(super) fn with_fetch_slot<T>(
        &self,
        language: &str,
        work: impl FnOnce() -> Result<T, LockPoisoned>,
    ) -> Result<FetchSlot<T>, LockPoisoned> {
        match self.check_eligibility(language)? {
            SlotEligibility::Cached => return Ok(FetchSlot::Ran(work()?)),
            SlotEligibility::Blocked => return Ok(FetchSlot::Blocked),
            SlotEligibility::NeedsFetch => {}
        }
        let guard = crate::util::lock(&self.fetch_lock, "grammar_fetch_single_flight")?;
        match self.check_eligibility(language)? {
            SlotEligibility::Cached => {
                drop(guard);
                Ok(FetchSlot::Ran(work()?))
            }
            SlotEligibility::Blocked => Ok(FetchSlot::Blocked),
            SlotEligibility::NeedsFetch => {
                let result = work()?;
                drop(guard);
                Ok(FetchSlot::Ran(result))
            }
        }
    }

    pub(super) fn load_failed_retry_due(&self) -> Result<bool, LockPoisoned> {
        Ok(!self.cooling_down()?)
    }
}

/// Process-wide grammar-fetch gate used in production.
pub(super) static GLOBAL_GATE: LazyLock<GateState> = LazyLock::new(GateState::new);

#[cfg(not(test))]
fn with_current_gate<T>(
    run: impl FnOnce(&GateState) -> Result<T, LockPoisoned>,
) -> Result<T, LockPoisoned> {
    run(&GLOBAL_GATE)
}

#[cfg(test)]
fn with_current_gate<T>(
    run: impl FnOnce(&GateState) -> Result<T, LockPoisoned>,
) -> Result<T, LockPoisoned> {
    test_controls::with_test_gate(run)
}

/// Records that a grammar fetch just failed, closing the gate.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the gate's state was poisoned.
pub(crate) fn record_fetch_failure() -> Result<(), LockPoisoned> {
    with_current_gate(GateState::record_fetch_failure)
}

/// Runs `work` for `language`, serializing concurrent uncached grammar fetches
/// behind a process-wide single-flight lock.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the gate or single-flight lock was poisoned.
pub(crate) fn with_fetch_slot<T>(
    language: &str,
    work: impl FnOnce() -> Result<T, LockPoisoned>,
) -> Result<FetchSlot<T>, LockPoisoned> {
    with_current_gate(|gate| gate.with_fetch_slot(language, work))
}

/// True when a stored `LoadFailed` row should be re-parsed now.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the gate's state was poisoned.
pub(crate) fn load_failed_retry_due() -> Result<bool, LockPoisoned> {
    with_current_gate(GateState::load_failed_retry_due)
}

#[cfg(test)]
pub(crate) mod test_controls {
    //! Test-only clock/cache hooks and thread-isolated [`super::GateState`] slots.

    use super::{GRAMMAR_FETCH_COOLDOWN, GateState};
    use crate::util::LockPoisoned;
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
    use std::time::Duration;

    const CACHE_HOST: u8 = 0;
    const CACHE_FORCED_UNCACHED: u8 = 1;
    const CACHE_FORCED_CACHED: u8 = 2;

    /// One second past the cool-down; checked at compile time so it cannot
    /// overflow at run time.
    pub(super) const PAST_COOLDOWN_SECS: u64 = match GRAMMAR_FETCH_COOLDOWN.as_secs().checked_add(1)
    {
        Some(secs) => secs,
        None => u64::MAX,
    };

    /// Fake clock skew and grammar-cache override attached to [`GateState`] in
    /// `#[cfg(test)]` builds only.
    pub(super) struct TestHooks {
        skew_secs: AtomicU64,
        cache_mode: AtomicU8,
    }

    impl TestHooks {
        pub(super) fn host_backed() -> Self {
            Self {
                skew_secs: AtomicU64::new(0),
                cache_mode: AtomicU8::new(CACHE_HOST),
            }
        }

        pub(super) fn isolated_uncached() -> Self {
            Self {
                skew_secs: AtomicU64::new(0),
                cache_mode: AtomicU8::new(CACHE_FORCED_UNCACHED),
            }
        }

        pub(super) fn skew(&self) -> Duration {
            Duration::from_secs(self.skew_secs.load(Ordering::Acquire))
        }

        pub(super) fn grammar_cached(&self, language: &str) -> bool {
            match self.cache_mode.load(Ordering::Acquire) {
                CACHE_FORCED_UNCACHED => false,
                CACHE_FORCED_CACHED => true,
                _ => tree_sitter_language_pack::has_parser(language),
            }
        }

        pub(super) fn advance_secs(&self, secs: u64) {
            let mut current = self.skew_secs.load(Ordering::Acquire);
            loop {
                // KEEP saturating: a skew past u64::MAX seconds is past every cool-down, so the clamped skew gives the same answer
                let next = current.saturating_add(secs);
                match self.skew_secs.compare_exchange_weak(
                    current,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
        }

        pub(super) fn set_cached(&self, cached: bool) {
            let mode = if cached {
                CACHE_FORCED_CACHED
            } else {
                CACHE_FORCED_UNCACHED
            };
            self.cache_mode.store(mode, Ordering::Release);
        }
    }

    thread_local! {
        static TEST_GATE: RefCell<Arc<GateState>> =
            RefCell::new(Arc::new(GateState::new_isolated()));
    }

    pub(super) fn with_test_gate<T>(
        run: impl FnOnce(&GateState) -> Result<T, LockPoisoned>,
    ) -> Result<T, LockPoisoned> {
        let gate = current_gate();
        run(&gate)
    }

    /// Moves this thread's gate clock past the cool-down.
    pub(crate) fn elapse_cooldown() {
        current_gate().hooks.advance_secs(PAST_COOLDOWN_SECS);
    }

    /// Makes `with_fetch_slot` treat every grammar as already cached (or not).
    pub(crate) fn set_grammar_cached(cached: bool) {
        current_gate().hooks.set_cached(cached);
    }

    /// Captures this thread's gate so spawned `file_pool` workers share it.
    pub(crate) fn current_gate() -> Arc<GateState> {
        TEST_GATE.with(|cell| Arc::clone(&cell.borrow()))
    }

    /// Installs `gate` on a spawned worker thread.
    pub(crate) fn install_gate(gate: Arc<GateState>) {
        TEST_GATE.with(|cell| *cell.borrow_mut() = gate);
    }
}

#[cfg(test)]
#[path = "fetch_gate_tests.rs"]
mod tests;
