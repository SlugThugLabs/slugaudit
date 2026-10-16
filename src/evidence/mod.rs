//! Neutral evidence normalization: turns the language pack's generic
//! `process()` output into SlugAudit's own typed evidence records.

mod calls;
mod generic_imports;
mod normalize;
mod normalize_builders;
mod sql;

/// How deep SlugAudit's own tree walkers descend before they stop.
///
/// The walkers recurse over the raw parse tree, whose depth is a property of
/// the file's content: a file nesting 6 000 parentheses parses to a
/// 6 000-deep tree, and unbounded recursion over it overflows the thread
/// stack — which ABORTS the process rather than raising a catchable panic, so
/// the tool call never returns and the caller's session dies with it.
///
/// 512 matches the bound the language pack already applies to its own
/// traversal (`ts_pack::intel` truncates at `max_depth=512` and reports
/// `skipped_nodes`), so any file the pack considers safe is one these walkers
/// finish. Exceeding it yields a truncated extraction, reported as
/// `ExtractionCompleteness::Partial` — never a crash.
pub(super) const MAX_WALK_DEPTH: u32 = 512;

pub use normalize::extract;

pub use sql::{EvidenceRow, to_row};
