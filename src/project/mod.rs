//! Project root resolution and activation. `.planning/slugaudit/` existing
//! is the only on/off control in the whole product: its presence enables
//! SlugAudit for a project, its absence disables it. `enable`/`disable`
//! are the one supported way to create or remove that marker — exposed to
//! the AI via the `project_control` MCP tool (`src/tools/project_control.rs`), not via any CLI command.

mod activation;
mod database_path;
mod root;

pub use activation::{ActivationError, activation_dir, disable, enable, find_project_root};
pub use database_path::database_path;
pub use root::{ProjectRoot, RootError};
