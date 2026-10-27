//! The immutable result store: a materialised answer is written once into a
//! private SQLite file (unlinked at creation, so the open descriptor is the
//! only reference) and then paged from there by a cursor, whatever happens to
//! the live index meanwhile.
//!
//! Layout under `<runtime_dir>/results/`: `<session>.lock` (advisory lock held
//! for the store's life, taken before any result file exists) and, only for
//! the instant before the unlink, `<session>-<result>.db`. A result is
//! addressable only once its writer committed `meta.state = 'complete'` and the
//! registry admitted it.
//!
//! Invariants: the sum of admitted bytes never exceeds the budget; an active
//! result (unfinished, served within the grace period) is never reclaimed;
//! delivered results go first, then idle ones, oldest first; the map lock is
//! never held across SQL or file I/O.
//!
//! The store knows nothing of tools, MCP or wire codes. The tool layer maps
//! its errors: `Unavailable` -> `result_store_unavailable`, `Full` ->
//! `result_store_full`, `TooLarge` -> `result_too_large`, `Expired` ->
//! `result_expired`, `Unknown` -> `unknown_cursor`. Supported on Unix; elsewhere
//! `open` returns `Unavailable`.

mod cell;
mod cursor;
mod error;
mod fault;
mod owner;
mod platform;
mod reader;
mod registry;
mod result_file;
mod stats;
mod store;
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
pub use owner::SweepOutcome;
pub use reader::{PageRead, TypedPage};
pub use registry::{ResultClass, ResultMeta};
pub use stats::{ClassStats, StoreStats};
pub use store::{Clock, ResultStore, ResultStoreLimits};
pub use writer::{ResultHandle, ResultSpec, ResultWriter};
