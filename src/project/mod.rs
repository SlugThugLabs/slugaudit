//! Project root resolution and activation. The directory
//! `.planning/slugaudit/` is the only on/off control in the whole product:
//! its presence enables SlugAudit for a project, its absence disables it.
//! Everything inside it is scratch: `project.db` is rebuilt on demand and
//! each `findings-<id>.db` lasts one SlugAudit process. `enable`/`disable` are the one
//! supported way to create or remove that directory, exposed to the AI via
//! the `project_control` MCP tool (`src/tools/project_control.rs`), not via
//! any CLI command.

mod activation;
mod database_path;
mod findings_session;
mod findings_sweep;
mod gitignore;
mod global_config;
mod mechanic_entry;
mod mechanic_scan_cache;
mod root;
mod session_mechanics;
mod session_profile;

pub use activation::{
    ActivationError, DisableReport, GitignoreOutcome, activation_dir, disable, enable,
    ensure_gitignore, find_project_root, resolve_project,
};
pub use database_path::database_path;
pub use findings_session::{begin_process_session, session_findings_path};
#[cfg(test)]
pub(crate) use findings_session::{
    forget_root_for_test, put_session_for_test, take_session_for_test,
};
#[cfg(test)]
pub(crate) use global_config::set_global_config_path_for_test;
pub use global_config::{ConfigError, GlobalConfig, ProjectLimitsError, effective_limits};
pub use mechanic_entry::MechanicEntry;
pub use mechanic_scan_cache::{
    CacheStats, CachedRow, DEFAULT_MECHANIC_CACHE_BYTES, ENTRY_OVERHEAD_BYTES, MechanicScanCache,
    ROW_OVERHEAD_BYTES, ScanKey,
};
pub use root::{ProjectRoot, RootError};
pub use session_mechanics::{MAX_SESSION_MECHANICS, SessionMechanics};
pub use session_profile::{session_profile, set_session_profile};

/// Test fixture: an empty owner-only `project.db` in `activation` (a
/// `.planning/slugaudit` directory), which SQLite opens as a fresh database.
#[cfg(test)]
pub(crate) fn write_database_marker(activation: &std::path::Path) {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(activation.join("project.db"))
        .expect("create database marker");
}

/// Test fixture: creates `<root>/.planning/slugaudit` with an empty
/// `project.db`, an active project whose first call builds the index.
#[cfg(test)]
pub(crate) fn activate_for_test(root: &std::path::Path) {
    let activation = root.join(".planning").join("slugaudit");
    std::fs::create_dir_all(&activation).expect("create activation dir");
    write_database_marker(&activation);
}
