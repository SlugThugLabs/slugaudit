//! The pack-extraction boundary for `analyze`, and its test seam.
//!
//! Production calls the pack directly. Unit tests can force a pack error
//! through [`forced_failure`], and count how often the pack loader was
//! reached, so the download-failure and cool-down paths run with no network
//! and without touching the user's grammar cache.
use crate::model::EvidenceItem;
use tree_sitter_language_pack::Error as PackError;

#[cfg(not(test))]
pub(super) fn run_extraction(
    language: &str,
    content: &str,
) -> Result<(Vec<EvidenceItem>, bool), PackError> {
    crate::evidence::extract(language, content)
}

#[cfg(test)]
pub(super) fn run_extraction(
    language: &str,
    content: &str,
) -> Result<(Vec<EvidenceItem>, bool), PackError> {
    forced_failure::count_call();
    if let Some(make_error) = forced_failure::current() {
        return Err(make_error());
    }
    crate::evidence::extract(language, content)
}

/// Test-only injection of a pack error into [`run_extraction`], scoped to
/// the calling thread so parallel tests cannot observe each other's.
#[cfg(test)]
pub(super) mod forced_failure {
    use super::PackError;
    use std::cell::Cell;

    thread_local! {
        static FORCED: Cell<Option<fn() -> PackError>> = const { Cell::new(None) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn current() -> Option<fn() -> PackError> {
        FORCED.with(Cell::get)
    }

    pub(super) fn count_call() {
        CALLS.with(|calls| calls.set(calls.get() + 1));
    }

    /// How many times this thread reached the pack loader.
    pub(crate) fn calls() -> usize {
        CALLS.with(Cell::get)
    }

    /// Restores "no forced failure" when dropped.
    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            FORCED.with(|forced| forced.set(None));
        }
    }

    /// Makes every extraction on this thread fail with `make_error()` until
    /// the returned guard drops.
    pub(crate) fn force(make_error: fn() -> PackError) -> Guard {
        FORCED.with(|forced| forced.set(Some(make_error)));
        Guard
    }
}
