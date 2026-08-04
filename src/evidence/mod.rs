//! Neutral evidence normalization: turns the language pack's generic
//! `process()` output into SlugAudit's own typed evidence records.

mod normalize;
mod normalize_builders;
mod sql;

pub use normalize::extract;
pub use sql::{EvidenceRow, to_row};
