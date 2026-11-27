//! Deterministic one-shot barriers for testing the race windows of a
//! publish attempt, without sleeps or timing assumptions.

use std::path::Path;

/// Where in a publish attempt a hook fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RacePoint {
    /// After the discovery walk listed the files, before Pass 1 hashes them.
    AfterDiscovery,
    /// After Pass 1 and Pass 2 work finished, immediately before opening
    /// `BEGIN IMMEDIATE` on `project.db` (or checking the baseline on the
    /// no-change path).
    BeforeCommit,
}

#[cfg(test)]
type Outcome = std::sync::Arc<std::sync::Mutex<Option<slugaudit_test_check::TestResult>>>;

#[cfg(test)]
type Hook = Box<dyn FnOnce() -> slugaudit_test_check::TestResult + Send>;

#[cfg(test)]
type Hooks = std::collections::HashMap<(std::path::PathBuf, RacePoint), (Hook, Outcome)>;

#[cfg(test)]
static HOOKS: std::sync::Mutex<Option<Hooks>> = std::sync::Mutex::new(None);

/// Handle for checking that an armed race hook fired and succeeded.
#[cfg(test)]
#[must_use = "a race test must check that its hook fired and succeeded"]
pub(crate) struct HookHandle {
    outcome: Outcome,
}

#[cfg(test)]
impl HookHandle {
    pub(crate) fn outcome(&self) -> slugaudit_test_check::TestResult {
        use slugaudit_test_check::Check;
        let fired = crate::util::lock(&self.outcome, "test_race_hook_outcome")
            .check("lock the race hook outcome")?
            .take();
        fired.check("the race hook fired")?
    }
}

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

#[cfg(not(test))]
pub(crate) fn fire(_root: &Path, _point: RacePoint) {}

#[cfg(test)]
#[path = "race_hook_tests.rs"]
mod tests;
