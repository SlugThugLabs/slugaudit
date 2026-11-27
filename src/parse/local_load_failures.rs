//! Session-permanent registry of languages whose grammar failed a local
//! (non-fetch) load in this server process.
//!
//! Unlike remote fetch failures (which cool down for ten minutes across all
//! uncached grammars), a corrupt or incompatible dynamic library on disk will
//! not heal until the cache changes or the server restarts. Recording those
//! languages here prevents unchanged `LoadFailed` rows from triggering an
//! infinite re-parse and new-revision storm on every tool call.

use crate::util::LockPoisoned;
use std::collections::BTreeSet;
use std::sync::{LazyLock, Mutex};

const LOCK_NAME: &str = "grammar_local_load_failures";

/// Thread-safe set of language names that suffered a non-fetch load failure.
pub(crate) struct LocalLoadFailures {
    languages: Mutex<BTreeSet<String>>,
}

impl LocalLoadFailures {
    /// Creates an empty failure set.
    pub(crate) fn new() -> Self {
        Self {
            languages: Mutex::new(BTreeSet::new()),
        }
    }

    fn with_set<T>(&self, op: impl FnOnce(&mut BTreeSet<String>) -> T) -> Result<T, LockPoisoned> {
        Ok(op(&mut *crate::util::lock(&self.languages, LOCK_NAME)?))
    }

    /// Marks `language` as having suffered a permanent local load failure.
    pub(crate) fn record(&self, language: &str) -> Result<(), LockPoisoned> {
        self.with_set(|set| {
            set.insert(language.to_owned());
        })
    }

    /// True when `language` is recorded in this failure set.
    pub(crate) fn contains(&self, language: &str) -> Result<bool, LockPoisoned> {
        self.with_set(|set| set.contains(language))
    }
}

static GLOBAL_FAILURES: LazyLock<LocalLoadFailures> = LazyLock::new(LocalLoadFailures::new);

#[cfg(test)]
thread_local! {
    static TEST_FAILURES: std::cell::RefCell<std::sync::Arc<LocalLoadFailures>> =
        std::cell::RefCell::new(std::sync::Arc::new(LocalLoadFailures::new()));
}

#[cfg(not(test))]
fn with_current<T>(
    run: impl FnOnce(&LocalLoadFailures) -> Result<T, LockPoisoned>,
) -> Result<T, LockPoisoned> {
    run(&GLOBAL_FAILURES)
}

#[cfg(test)]
fn with_current<T>(
    run: impl FnOnce(&LocalLoadFailures) -> Result<T, LockPoisoned>,
) -> Result<T, LockPoisoned> {
    let set = current_test_set();
    run(&set)
}

#[cfg(test)]
pub(crate) fn current_test_set() -> std::sync::Arc<LocalLoadFailures> {
    TEST_FAILURES.with(|cell| std::sync::Arc::clone(&cell.borrow()))
}

#[cfg(test)]
pub(crate) fn install_test_set(set: std::sync::Arc<LocalLoadFailures>) {
    TEST_FAILURES.with(|cell| *cell.borrow_mut() = set);
}

/// Records a permanent local (non-fetch) grammar load failure for `language`.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the failure set mutex was poisoned.
pub(crate) fn record_local_load_failure(language: &str) -> Result<(), LockPoisoned> {
    with_current(|set| set.record(language))
}

/// True when `language` previously suffered a permanent local (non-fetch) load
/// failure in this process.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the failure set mutex was poisoned.
pub(crate) fn is_locally_failed(language: &str) -> Result<bool, LockPoisoned> {
    with_current(|set| set.contains(language))
}

#[cfg(test)]
#[path = "local_load_failures_tests.rs"]
mod tests;
