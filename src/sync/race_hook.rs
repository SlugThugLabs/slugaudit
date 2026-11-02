//! Deterministic one-shot barrier for testing the race window between
//! sampling and publishing, without sleeps or timing assumptions.
use std::path::Path;

// A map, not a single slot: `cargo test` runs different `#[test]` fns
// concurrently by default, and each race test calls `set` near the start of
// its body. A single slot would lose hooks under real parallelism — one
// test's `set` would silently drop (and never fire) a different, not-yet-
// consumed test's still-armed closure. Keying by root path, which every
// test gets a fresh unique tempdir for, makes concurrent race tests fully
// independent of each other regardless of scheduling.
#[cfg(test)]
type Hooks = std::collections::HashMap<
    std::path::PathBuf,
    Box<dyn FnOnce() -> slugaudit_test_check::TestResult + Send>,
>;

#[cfg(test)]
static HOOKS: std::sync::Mutex<Option<Hooks>> = std::sync::Mutex::new(None);

/// Arms a one-shot hook that fires the next time [`fire`] is called for the
/// same `root`. Both sides key by the canonical root: the sync path passes
/// [`fire`] a canonical root while `publish()` callers pass the test's temp
/// path, which is not canonical on macOS (`/var` → `/private/var`). A hook
/// keyed by only one spelling never fires, and the publish race tests wait
/// forever. A hook that fails reports its error through [`fire`], which logs
/// it once; the publish under test cannot carry it.
#[cfg(test)]
pub(crate) fn set(
    root: &Path,
    hook: impl FnOnce() -> slugaudit_test_check::TestResult + Send + 'static,
) -> slugaudit_test_check::TestResult {
    use slugaudit_test_check::Check;
    let key = std::fs::canonicalize(root).check("canonicalize the hook root")?;
    crate::util::lock(&HOOKS, "test_race_hooks")
        .check("lock the race hooks")?
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(key, Box::new(hook));
    Ok(())
}

/// Fires the hook armed for `root`, if any, and removes it; a no-op for any
/// root nobody armed a hook for.
#[cfg(test)]
pub(crate) fn fire(root: &Path) {
    let hook = match take_hook(root) {
        Ok(hook) => hook,
        Err(error) => {
            tracing::error!(error = %error, "a test race hook could not be looked up");
            return;
        }
    };
    if let Some(hook) = hook
        && let Err(error) = hook()
    {
        tracing::error!(error = %error, "a test race hook failed");
    }
}

/// Removes and returns the hook armed for `root`, if any.
#[cfg(test)]
fn take_hook(
    root: &Path,
) -> slugaudit_test_check::TestResult<
    Option<Box<dyn FnOnce() -> slugaudit_test_check::TestResult + Send>>,
> {
    use slugaudit_test_check::Check;
    let key = std::fs::canonicalize(root).check("canonicalize the hook root")?;
    Ok(crate::util::lock(&HOOKS, "test_race_hooks")
        .check("lock the race hooks")?
        .as_mut()
        .and_then(|hooks| hooks.remove(&key)))
}

/// Release builds have no hooks: this compiles to nothing.
#[cfg(not(test))]
pub(crate) fn fire(_root: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, TestResult};

    #[test]
    fn a_hook_armed_through_a_symlinked_root_fires_for_either_spelling() -> TestResult {
        let real = tempfile::tempdir().check("real root")?;
        let links = tempfile::tempdir().check("link dir")?;
        let link = links.path().join("root");
        std::os::unix::fs::symlink(real.path(), &link).check("symlink root")?;
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first = std::sync::Arc::clone(&fired);
        set(&link, move || {
            first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })?;
        fire(real.path());
        let second = std::sync::Arc::clone(&fired);
        set(real.path(), move || {
            second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })?;
        fire(&link);

        assert_eq!(
            fired.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "set and fire must agree on the root whichever spelling each side uses"
        );
        Ok(())
    }
}
