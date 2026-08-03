//! Project root resolution and activation. `.planning/slugaudit/` existing
//! is the only human-facing control in the whole product: its presence
//! enables SlugAudit for a project, its absence disables it. `enable`/
//! `disable` are the one supported way to create or remove that marker —
//! exposed to a human via the `slugaudit-mcp-rust enable`/`disable` CLI
//! commands (`src/cli.rs`).

mod activation;
mod database_path;
mod root;

pub use activation::{ActivationError, activation_dir, disable, enable, find_project_root};
pub use database_path::database_path;
pub use root::{ProjectRoot, RootError};
