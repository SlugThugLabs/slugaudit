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
/// same `root`.
#[cfg(test)]
pub(crate) fn set(root: &Path, hook: impl FnOnce() + Send + 'static) {
    let mut guard = HOOKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(root.to_path_buf(), Box::new(hook));
}

/// A no-op outside tests (this module is `cfg(test)`-only besides this one
/// always-compiled entry point, so it compiles to nothing in release
/// builds) and a no-op for any root nobody armed a hook for.
pub(crate) fn fire(_root: &Path) {
    #[cfg(test)]
    {
        let hook = HOOKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .and_then(|hooks| hooks.remove(_root));
        if let Some(hook) = hook {
            hook();
        }
    }
}
