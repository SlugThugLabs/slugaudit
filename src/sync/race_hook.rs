//! Deterministic one-shot barriers for testing the race windows of a
//! publish attempt, without sleeps or timing assumptions.
use std::path::Path;

/// Where in a publish attempt a hook fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RacePoint {
    /// After the discovery walk listed the files, before Pass 1 hashes them.
    AfterDiscovery,
    /// After the attempt read the tree, before it checks the baseline and
    /// writes (both the no-change path and the commit path).
    BeforeCommit,
}

/// A hook that fails, or never fires, must fail the test that armed it:
/// the publish under test cannot carry the hook's result, so [`fire`]
/// stores it here for the test to check.
#[cfg(test)]
type Outcome = std::sync::Arc<std::sync::Mutex<Option<slugaudit_test_check::TestResult>>>;

#[cfg(test)]
type Hook = Box<dyn FnOnce() -> slugaudit_test_check::TestResult + Send>;

// A map, not a single slot: `cargo test` runs different `#[test]` fns
// concurrently by default, and each race test calls `set` near the start of
// its body. A single slot would lose hooks under real parallelism — one
// test's `set` would silently drop (and never fire) a different, not-yet-
// consumed test's still-armed closure. Keying by root path, which every
// test gets a fresh unique tempdir for, makes concurrent race tests fully
// independent of each other regardless of scheduling.
#[cfg(test)]
type Hooks = std::collections::HashMap<(std::path::PathBuf, RacePoint), (Hook, Outcome)>;

#[cfg(test)]
static HOOKS: std::sync::Mutex<Option<Hooks>> = std::sync::Mutex::new(None);

/// What [`set`] returns: the way to read the armed hook's result.
#[cfg(test)]
#[must_use = "a race test must check that its hook fired and succeeded"]
pub(crate) struct HookHandle {
    outcome: Outcome,
}

#[cfg(test)]
impl HookHandle {
    /// The hook's own result once it fired.
    ///
    /// # Errors
    ///
    /// The hook's error, or an error when the hook never fired: a race test
    /// whose race did not happen proves nothing.
    pub(crate) fn outcome(&self) -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::Check;
        let fired = crate::util::lock(&self.outcome, "test_race_hook_outcome")
            .check("lock the race hook outcome")?
            .take();
        fired.check("the race hook fired")?
    }
}

/// Arms a one-shot hook that fires the next time [`fire`] is called for the
/// same `root` and `point`. Both sides key by the canonical root: the sync
/// path passes [`fire`] a canonical root while `publish()` callers pass the
/// test's temp path, which is not canonical on macOS (`/var` →
/// `/private/var`). A hook keyed by only one spelling never fires, and the
/// publish race tests wait forever.
///
/// # Errors
///
/// When the root cannot be canonicalized or the hook map is poisoned.
#[cfg(test)]
pub(crate) fn set(
    root: &Path,
    point: RacePoint,
    hook: impl FnOnce() -> slugaudit_test_check::TestResult + Send + 'static,
) -> slugaudit_test_check::TestResult<HookHandle> {
    use slugaudit_test_check::Check;
    let key = std::fs::canonicalize(root).check("canonicalize the hook root")?;
    let outcome = Outcome::default();
    crate::util::lock(&HOOKS, "test_race_hooks")
        .check("lock the race hooks")?
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            (key, point),
            (Box::new(hook), std::sync::Arc::clone(&outcome)),
        );
    Ok(HookHandle { outcome })
}

/// Fires the hook armed for `root` and `point`, if any, removes it, and
/// stores its result for [`HookHandle::outcome`]; a no-op for any root and
/// point nobody armed a hook for. A failed lookup reaches no handle, so it
/// is logged; the armed test then sees its hook as never fired.
#[cfg(test)]
pub(crate) fn fire(root: &Path, point: RacePoint) {
    let armed = match take_hook(root, point) {
        Ok(armed) => armed,
        Err(error) => {
            tracing::error!(error = %error, "a test race hook could not be looked up");
            return;
        }
    };
    let Some((hook, outcome)) = armed else {
        return;
    };
    let result = hook();
    match crate::util::lock(&outcome, "test_race_hook_outcome") {
        Ok(mut slot) => *slot = Some(result),
        Err(poisoned) => tracing::error!(
            error = %poisoned,
            "a test race hook's result could not be stored",
        ),
    }
}

/// Removes and returns the hook armed for `root` and `point`, if any.
#[cfg(test)]
fn take_hook(
    root: &Path,
    point: RacePoint,
) -> slugaudit_test_check::TestResult<Option<(Hook, Outcome)>> {
    use slugaudit_test_check::Check;
    let key = std::fs::canonicalize(root).check("canonicalize the hook root")?;
    Ok(crate::util::lock(&HOOKS, "test_race_hooks")
        .check("lock the race hooks")?
        .as_mut()
        .and_then(|hooks| hooks.remove(&(key, point))))
}

/// Release builds have no hooks: this compiles to nothing.
#[cfg(not(test))]
pub(crate) fn fire(_root: &Path, _point: RacePoint) {}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, CheckErr, TestResult};

    #[test]
    fn a_hook_armed_through_a_symlinked_root_fires_for_either_spelling() -> TestResult {
        let real = tempfile::tempdir().check("real root")?;
        let links = tempfile::tempdir().check("link dir")?;
        let link = links.path().join("root");
        std::os::unix::fs::symlink(real.path(), &link).check("symlink root")?;
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first = std::sync::Arc::clone(&fired);
        let first_handle = set(&link, RacePoint::BeforeCommit, move || {
            first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })?;
        fire(real.path(), RacePoint::BeforeCommit);
        let second = std::sync::Arc::clone(&fired);
        let second_handle = set(real.path(), RacePoint::BeforeCommit, move || {
            second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })?;
        fire(&link, RacePoint::BeforeCommit);

        assert_eq!(
            fired.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "set and fire must agree on the root whichever spelling each side uses"
        );
        first_handle.outcome()?;
        second_handle.outcome()
    }

    #[test]
    fn a_hook_fires_only_at_its_own_point() -> TestResult {
        let root = tempfile::tempdir().check("root")?;
        let handle = set(root.path(), RacePoint::AfterDiscovery, || Ok(()))?;

        fire(root.path(), RacePoint::BeforeCommit);
        assert!(
            handle.outcome().is_err(),
            "a hook must not fire at another point"
        );
        fire(root.path(), RacePoint::AfterDiscovery);
        handle.outcome()
    }

    #[test]
    fn a_failed_hook_reaches_its_handle() -> TestResult {
        let root = tempfile::tempdir().check("root")?;
        let handle = set(root.path(), RacePoint::BeforeCommit, || {
            Err::<(), _>("hook step failed").check("the probe step")
        })?;

        fire(root.path(), RacePoint::BeforeCommit);

        let error = handle
            .outcome()
            .check_err("a failed hook must fail its handle")?;
        assert!(
            error.to_string().contains("hook step failed"),
            "the handle must carry the hook's own error: {error}"
        );
        Ok(())
    }

    #[test]
    fn a_hook_that_never_fired_fails_its_handle() -> TestResult {
        let root = tempfile::tempdir().check("root")?;
        let handle = set(root.path(), RacePoint::BeforeCommit, || Ok(()))?;

        let error = handle
            .outcome()
            .check_err("an unfired hook must fail its handle")?;
        assert!(
            error.to_string().contains("the race hook fired"),
            "got: {error}"
        );
        Ok(())
    }
}
