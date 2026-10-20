//! Test-only per-root walk counters for the stat sweep.
//!
//! The Healthy path's single-walk contract is asserted by counting walks,
//! which must not exist in the shipped runtime. Kept beside `sweep` but in
//! its own file so `sweep.rs` stays under the small-file budget.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static WALK_COUNTERS: Mutex<Option<HashMap<PathBuf, Arc<AtomicUsize>>>> = Mutex::new(None);

/// Arms walk counting for `root` and returns the counter [`super::sweep`]
/// bumps on every walk of that root.
pub(super) fn count_walks(root: &Path) -> Arc<AtomicUsize> {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut guard = crate::util::lock_or_recover(&WALK_COUNTERS, "walk_counters");
    guard
        .get_or_insert_with(HashMap::new)
        .insert(crate::util::canonical_or_self(root), Arc::clone(&counter));
    counter
}

/// Bumps the armed counter for `root`, if any; a no-op for any root nobody
/// armed a counter for. Counting exists only in test builds.
pub(super) fn note_walk(root: &Path) {
    let guard = crate::util::lock_or_recover(&WALK_COUNTERS, "walk_counters");
    // Canonical like the watcher's keys, so a counter armed with a tempdir
    // path matches the walk's resolved root.
    if let Some(counter) = guard
        .as_ref()
        .and_then(|counters| counters.get(&crate::util::canonical_or_self(root)))
    {
        counter.fetch_add(1, Ordering::SeqCst);
    }
}
