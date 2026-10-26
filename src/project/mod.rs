//! Project root resolution and activation. `.planning/slugaudit/` existing
//! is the only on/off control in the whole product: its presence enables
//! SlugAudit for a project, its absence disables it. `enable`/`disable`
//! are the one supported way to create or remove that marker — exposed to
//! the AI via the `project_control` MCP tool (`src/tools/project_control.rs`), not via any CLI command.

mod activation;
mod activation_lock;
mod config;
mod database_path;
mod gitignore;
mod mechanics_config;
mod root;
mod session_mechanics;

pub use activation::{
    ActivationError, GitignoreOutcome, activation_dir, disable, enable, ensure_gitignore,
    find_project_root, resolve_project,
};
pub use config::{ConfigError, ConfigWriteError, ProjectConfig};
pub use database_path::database_path;
pub use mechanics_config::{
    InvalidPersistedEntry, MAX_PERSISTED_MECHANICS, MECHANICS_SECTION_VERSION, MechanicEntry,
    MechanicsWriteError, PersistedMechanics, load_persisted, persist_entry, remove_persisted,
};
pub use root::{ProjectRoot, RootError};
pub use session_mechanics::{MAX_SESSION_MECHANICS, SessionMechanics};
