//! Immutable multi-page result store: overflow answers larger than Page 1 are
//! materialized once into an unlinked scratch SQLite file and paged by cursor.
//!
//! Layout under `<runtime_dir>/results/`: `<session>.lock` (advisory lock held
//! for the store's life so startup sweeps skip live sessions) and temporary
//! `<session>-<result>.db` files unlinked immediately after open on Unix so
//! nothing lingers on disk if the process is killed.

mod cell;
mod cursor;
mod error;
mod owner;
mod reader;
mod registry;
mod result_file;
mod store;
mod sweep;
mod writer;

#[cfg(test)]
#[path = "admission_tests.rs"]
mod admission_tests;
#[cfg(test)]
#[path = "cursor_tests.rs"]
mod cursor_tests;
#[cfg(test)]
#[path = "fidelity_tests.rs"]
mod fidelity_tests;
#[cfg(test)]
#[path = "harness_tests.rs"]
mod harness_tests;
#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "paging_tests.rs"]
mod paging_tests;

pub use cell::{Cell, Materialize, NonFinite, StoredRow, Take};
pub use cursor::{Cursor, ResultId};
pub use error::ResultStoreError;
pub use reader::PageRead;
pub use registry::{ClassStats, ResultClass, ResultMeta, StoreStats};
pub use store::{Clock, ResultStore, ResultStoreLimits};
pub use sweep::SweepOutcome;
pub use writer::{ResultHandle, ResultSpec, ResultWriter};
