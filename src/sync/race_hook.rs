//! Deterministic one-shot barrier for testing the race window between
//! sampling and publishing, without sleeps or timing assumptions.
use std::path::Path;

// A map, not a single slot: `cargo test` runs different `#[test]` fns
// concurrently by default, and each race test calls `set` near the start of
// its body. A single-slot design lost hooks under real parallelism — one
// test's `set` would silently drop (and never fire) a different, not-yet-
// consumed test's still-armed closure. Keying by root path, which every
// test gets a fresh unique tempdir for, makes concurrent race tests fully
// independent of each other regardless of scheduling.
#[cfg(test)]
type Hooks = std::collections::HashMap<std::path::PathBuf, Box<dyn FnOnce() + Send>>;

#[cfg(test)]
static HOOKS: std::sync::Mutex<Option<Hooks>> = std::sync::Mutex::new(None);

/// Arms a one-shot hook that fires the next time [`fire`] is called for the
/// same `root`. Both sides key by the canonical root: the sync path passes
/// [`fire`] a canonical root while `publish()` callers pass the test's temp
/// path, which is not canonical on macOS (`/var` → `/private/var`). Keying
/// only one side made the publish race tests wait forever on macOS CI
/// (run 38949456424, 2026-10-16, cancelled at the 6-hour job limit).
#[cfg(test)]
pub(crate) fn set(root: &Path, hook: impl FnOnce() + Send + 'static) {
    let mut guard = HOOKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(crate::util::canonical_or_self(root), Box::new(hook));
}

/// A no-op outside tests (this module is `cfg(test)`-only besides this one
/// always-compiled entry point, so it compiles to nothing in release
/// builds) and a no-op for any root nobody armed a hook for.
#[cfg_attr(not(test), allow(unused_variables))]
pub(crate) fn fire(root: &Path) {
    #[cfg(test)]
    {
        let hook = HOOKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .and_then(|hooks| hooks.remove(&crate::util::canonical_or_self(root)));
        if let Some(hook) = hook {
            hook();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hook_armed_through_a_symlinked_root_fires_for_either_spelling() {
        let real = tempfile::tempdir().expect("real root");
        let links = tempfile::tempdir().expect("link dir");
        let link = links.path().join("root");
        std::os::unix::fs::symlink(real.path(), &link).expect("symlink root");
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first = std::sync::Arc::clone(&fired);
        set(&link, move || {
            first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        fire(real.path());
        let second = std::sync::Arc::clone(&fired);
        set(real.path(), move || {
            second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        fire(&link);

        assert_eq!(
            fired.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "set and fire must agree on the root whichever spelling each side uses"
        );
    }
}
