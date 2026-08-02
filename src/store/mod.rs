//! SQLite schema, connection management, and (later) typed repositories.
//! The store never reads project files; sync/tools own that.

mod connection;
mod migrations;

pub use connection::{StoreError, open_read_only, open_read_write};
pub use migrations::MigrationError;
