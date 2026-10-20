//! The completeness block's schema types: what the index does *not* contain.
//!
//! `report` used to present bare counts — `file_count` was
//! `count(*) FROM files` — with nothing saying whether a number was the whole
//! truth. A file discovered but skipped (oversize, unreadable) had no row at
//! all, so it was absent from every count; a file whose text was indexed but
//! never parsed was indistinguishable from one whose parser failed. The tools
//! contract is that every number is quotable as-is, so each of those gaps is
//! now a count in this block.
//!
//! Facts only, in the same register as the rest of `report`: no scores, no
//! severity, no ordering by importance. `skipped_by_reason` is ordered by the
//! reason code and the other breakdowns by their value text, so the ordering
//! carries no judgment a reader could mistake for a ranking.
//!
//! The definitions live here so `report_completeness` stays within the
//! source-size limit; it re-exports them, so the public path
//! (`tools::report_completeness::*`) is unchanged.

use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_kind` value.
pub struct FileKindCount {
    /// One of `indexed`, `binary`, `skipped` — `files.file_kind`.
    pub kind: String,
    /// Number of `files` rows carrying this `file_kind`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_class` value.
pub struct FileClassCount {
    /// One of `authored`, `generated`, `vendored` — `files.file_class`.
    pub class: String,
    /// Number of `files` rows carrying this `file_class`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one recorded skip reason.
pub struct SkipReasonCount {
    /// A short stable code from `files.skip_reason`: `oversize` (over the
    /// per-file import ceiling), `unreadable` (the read failed) or
    /// `stat_failed` (the stat failed, so `files.byte_len` is NULL: size
    /// unknown, not 0). Empty when a skipped row carries no recorded reason.
    pub reason: String,
    /// Number of skipped `files` rows with this `reason`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one extraction-completeness value.
pub struct CompletenessCount {
    /// One of `Full`, `Partial`, `ContentOnly`, `Unavailable`. `ContentOnly`
    /// means the file's text is in `files.content` but no parser ran over it
    /// — the file is readable even though it has no structural evidence.
    /// `Unavailable` means no content is retained at all (binary, or skipped).
    pub extraction_completeness: String,
    /// Number of `files` rows with this `extraction_completeness` value.
    pub count: i64,
}

/// Counts that say whether the rest of the response is the whole truth.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompletenessEnvelope {
    /// Every `files` row by `file_kind`. `indexed` is the set with content and
    /// metadata — its structural evidence may still be missing, which
    /// `indexed_files_without_evidence` counts; `binary` was sampled and
    /// deliberately not parsed; `skipped` was discovered but never read, so
    /// it has no content.
    pub file_kind_counts: Vec<FileKindCount>,
    /// Every `files` row by `file_class` — where the code came from, derived
    /// from the path alone (`sync::classify`): `authored` (the project's own
    /// source), `generated` (a machine-written file: a `generated` path token
    /// or a codegen suffix such as `.pb.go` / `.g.dart` / `.min.js`), or
    /// `vendored` (a third-party tree: `vendor/`, `node_modules/`, …). A
    /// mechanic whose rows are dominated by one class can be summarised in a
    /// sentence instead of paged row by row. `vendored` subsumes `generated`
    /// when a path matches both. A path heuristic, not a verdict: it says
    /// where a file came from, never whether it matters.
    pub file_class_counts: Vec<FileClassCount>,
    /// `files` rows with `file_kind = 'skipped'`, i.e. files that exist in the
    /// project but are not in the index.
    pub skipped_file_count: i64,
    /// Those skipped rows grouped by `skip_reason`. This is the "why" paired
    /// with `index_partial`. A file with no representable `files.path` (a
    /// non-UTF-8 name) cannot appear here; it is counted by
    /// `unrepresentable_path_count` instead.
    pub skipped_by_reason: Vec<SkipReasonCount>,
    /// Every `files` row by `extraction_completeness`, so "text present but
    /// not parsed" (`ContentOnly`) is visible as itself instead of vanishing
    /// into a parser-failure count.
    pub extraction_completeness_counts: Vec<CompletenessCount>,
    /// True when the index does not cover every file discovery found in the
    /// current tree: an existence row is `skipped`, or a discovered file's
    /// path has no UTF-8 representation and therefore no `files` row at all.
    /// False means every discovered file is represented by a row here.
    pub index_partial: bool,
    /// Discovered files whose project-relative path is not valid UTF-8, so
    /// no `files` row can represent them and they cannot be stored, skipped
    /// or named in this report. Sync persists this set during every full
    /// discovery and refreshes it from every incremental stat sweep, so the
    /// count is read from the index rather than from a walk run for this
    /// report. It is project-level — describing the tree as of the sync that
    /// produced this report, not a particular revision — and is included in
    /// `index_partial`.
    pub unrepresentable_path_count: i64,
    /// Files whose stored evidence was cut short by an evidence cap: each
    /// carries the `evidence:truncated` diagnostic appended by
    /// `sync::sample::apply_evidence_limits`. Their structural record is
    /// incomplete in a way this report cannot recover, so an agent counting
    /// `evidence_counts` for them is counting a floor, not a total.
    pub evidence_capped_file_count: i64,
    /// `indexed` rows with no evidence at all — content is present and
    /// readable, but the index holds nothing structural about them (a
    /// language with no loaded parser, or a file that parsed to nothing).
    /// Binary and skipped rows are excluded: no evidence is their expected
    /// state, not a gap.
    pub indexed_files_without_evidence: i64,
}
