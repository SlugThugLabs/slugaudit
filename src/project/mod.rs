//! Project root resolution and activation. `.planning/slugaudit/` existing
//! is the only human-facing control in the whole product: its presence
//! enables SlugAudit for a project, its absence disables it.

mod activation;
mod database_path;
mod root;

pub use activation::{ActivationError, activation_dir, find_project_root};
pub use database_path::database_path;
pub use root::{ProjectRoot, RootError};
