//! SQLite schema, connection management, and (later) typed repositories.
//! The store never reads project files; sync/tools own that.

mod connection;
mod migrations;
mod netfs;

#[cfg(test)]
#[path = "test_capture.rs"]
mod test_capture;

pub use connection::{StoreError, discard_corrupt_database, open_read_only, open_read_write};
pub use migrations::MigrationError;
