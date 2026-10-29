//! Project root resolution and activation. The regular file
//! `.planning/slugaudit/project.db` is the only on/off control in the whole
//! product: its presence enables SlugAudit for a project, its absence
//! disables it (the directory alone means nothing; `off` keeps `config.json`).
//! `enable`/`disable` are the one supported way to create or remove that state — exposed to
//! the AI via the `project_control` MCP tool (`src/tools/project_control.rs`), not via any CLI command.

mod activation;
mod activation_lock;
mod config;
mod config_cleanup;
mod config_store;
mod database_path;
mod gitignore;
mod mechanic_scan_cache;
mod mechanics_config;
mod root;
mod session_mechanics;

pub use activation::{
    ActivationError, DisableReport, GitignoreOutcome, activation_dir, disable, enable,
    ensure_gitignore, find_project_root, resolve_project,
};
pub use config::{ConfigError, ProjectConfig, ProjectLimitsError};
pub use config_store::ConfigWriteError;
pub use database_path::database_path;
pub use mechanic_scan_cache::{
    CacheStats, CachedRow, DEFAULT_MECHANIC_CACHE_BYTES, ENTRY_OVERHEAD_BYTES, MechanicScanCache,
    ROW_OVERHEAD_BYTES, ScanKey,
};
pub use mechanics_config::{
    InvalidPersistedEntry, MAX_PERSISTED_MECHANICS, MECHANICS_SECTION_VERSION, MechanicEntry,
    MechanicsWriteError, PersistedMechanics, load_persisted, persist_entry, remove_persisted,
    remove_persisted_at,
};
pub use root::{ProjectRoot, RootError};
pub use session_mechanics::{MAX_SESSION_MECHANICS, SessionMechanics};

/// Test fixture: the empty owner-only `project.db` that marks `activation`
/// (a `.planning/slugaudit` directory) as an active project.
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

/// Test fixture: creates `<root>/.planning/slugaudit` with its `project.db`
/// marker, the minimum shape of an active project.
#[cfg(test)]
pub(crate) fn activate_for_test(root: &std::path::Path) {
    let activation = root.join(".planning").join("slugaudit");
    std::fs::create_dir_all(&activation).expect("create activation dir");
    write_database_marker(&activation);
}
