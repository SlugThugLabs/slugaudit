//! Project root resolution, activation, session findings/mechanics/profile
//! state, and global/environment limit resolution.
//!
//! `.planning/slugaudit/` is the sole project activation switch (`project_control`
//! `on`/`off`). Everything inside it is disposable scratch.

mod activation;
mod findings_session;
mod gitignore;
mod global_config;
mod limits;
mod session_mechanics;
mod session_profile;

pub use activation::{
    ActivationError, DisableReport, GitignoreOutcome, ProjectRoot, RootError, activation_dir,
    database_path, disable, enable, ensure_gitignore, find_project_root, resolve_project,
};
pub use findings_session::{begin_process_session, session_findings_path};
#[cfg(test)]
pub(crate) use findings_session::{
    forget_root_for_test, put_session_for_test, take_session_for_test,
};
pub use global_config::{ConfigError, GlobalConfig};
#[cfg(test)]
pub(crate) use global_config::{GlobalConfigGuardForTest, set_global_config_path_for_test};
#[cfg(test)]
pub(crate) use limits::validate_startup_config_with;
pub use limits::{ProjectLimitsError, effective_limits, validate_startup_config};
pub use session_mechanics::{
    CompiledMechanic, MAX_SESSION_MECHANICS, MechanicEntry, SessionMechanics,
};
pub use session_profile::{clear_session_profile, session_profile, set_session_profile};

/// Test fixture: creates an empty `project.db` inside `.planning/slugaudit`.
#[cfg(test)]
pub(crate) fn write_database_marker(
    activation: &std::path::Path,
) -> slugaudit_test_check::TestResult {
    use slugaudit_test_check::Check;
    std::fs::File::create(activation.join("project.db")).check("create database marker")?;
    Ok(())
}

/// Test fixture: creates `<root>/.planning/slugaudit` with an empty `project.db`.
#[cfg(test)]
pub(crate) fn activate_for_test(root: &std::path::Path) -> slugaudit_test_check::TestResult {
    use slugaudit_test_check::Check;
    let activation = root.join(".planning").join("slugaudit");
    std::fs::create_dir_all(&activation).check("create activation dir")?;
    write_database_marker(&activation)?;
    Ok(())
}
