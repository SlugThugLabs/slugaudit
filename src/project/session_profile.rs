//! The audit profile the model chose for a project with `project_control on`,
//! held in memory for this process only. A new process starts with none, so
//! the profile from the environment, the global file or the default applies
//! until the model chooses again.

use crate::model::AuditProfile;
use crate::util::LockPoisoned;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Session profiles by canonical project root.
static PROFILES: Mutex<Option<HashMap<PathBuf, AuditProfile>>> = Mutex::new(None);

/// Records `profile` as the session profile of the project at `root`,
/// replacing an earlier choice.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the profile table was poisoned.
pub fn set_session_profile(root: &Path, profile: AuditProfile) -> Result<(), LockPoisoned> {
    crate::util::lock(&PROFILES, "session_profiles")?
        .get_or_insert_with(HashMap::new)
        .insert(root.to_path_buf(), profile);
    Ok(())
}

/// The session profile of the project at `root`, if the model chose one in
/// this process.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the profile table was poisoned.
pub fn session_profile(root: &Path) -> Result<Option<AuditProfile>, LockPoisoned> {
    Ok(crate::util::lock(&PROFILES, "session_profiles")?
        .as_ref()
        .and_then(|profiles| profiles.get(root).copied()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use slugaudit_test_check::{Check, TestResult};

    #[test]
    fn a_session_profile_is_per_project_and_the_last_choice_wins() -> TestResult {
        let first = Path::new("/session-profile-test/first");
        let second = Path::new("/session-profile-test/second");
        assert_eq!(session_profile(first).check("read first")?, None);

        set_session_profile(first, AuditProfile::Lean).check("choose lean")?;
        set_session_profile(first, AuditProfile::Audit).check("choose audit")?;

        assert_eq!(
            session_profile(first).check("read first")?,
            Some(AuditProfile::Audit)
        );
        assert_eq!(session_profile(second).check("read second")?, None);
        Ok(())
    }
}
