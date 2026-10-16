//! The completeness block of `report`: what the index does *not* contain.
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

use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_kind` value.
pub struct FileKindCount {
    /// One of `indexed`, `binary`, `skipped` — `files.file_kind`.
    pub kind: String,
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_class` value.
pub struct FileClassCount {
    /// One of `authored`, `generated`, `vendored` — `files.file_class`.
    pub class: String,
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one recorded skip reason.
pub struct SkipReasonCount {
    /// A short stable code from `files.skip_reason`: `oversize` (over the
    /// per-file import ceiling) or `unreadable` (the read failed). Empty when
    /// a skipped row carries no recorded reason.
    pub reason: String,
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
    pub count: i64,
}

/// Counts that say whether the rest of the response is the whole truth.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompletenessEnvelope {
    /// Every `files` row by `file_kind`. `indexed` is the set with content and
    /// structural evidence; `binary` was sampled and deliberately not parsed;
    /// `skipped` was discovered but never read, so it has no content.
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
    /// with `index_partial`; a file the walk could not even name (a non-UTF-8
    /// path) has no representable `files.path` and cannot appear here — it is
    /// visible only in the publish report and the server log.
    pub skipped_by_reason: Vec<SkipReasonCount>,
    /// Every `files` row by `extraction_completeness`, so "text present but
    /// not parsed" (`ContentOnly`) is visible as itself instead of vanishing
    /// into a parser-failure count.
    pub extraction_completeness_counts: Vec<CompletenessCount>,
    /// True when the index does not cover every file the last publish
    /// discovered — i.e. `skipped_file_count > 0`. False means every
    /// discovered file is represented by a row here.
    pub index_partial: bool,
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

/// Builds the envelope from the current revision's `files` table.
///
/// # Errors
///
/// Returns a database error if any of the grouped counts fails.
pub(super) fn build(connection: &Transaction<'_>) -> Result<CompletenessEnvelope, rusqlite::Error> {
    let mut kind_statement = connection
        .prepare("SELECT file_kind, count(*) FROM files GROUP BY file_kind ORDER BY file_kind")?;
    let file_kind_counts = kind_statement
        .query_map([], |row| {
            Ok(FileKindCount {
                kind: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(kind_statement);

    let mut class_statement = connection.prepare(
        "SELECT file_class, count(*) FROM files GROUP BY file_class ORDER BY file_class",
    )?;
    let file_class_counts = class_statement
        .query_map([], |row| {
            Ok(FileClassCount {
                class: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(class_statement);

    let mut reason_statement = connection.prepare(
        "SELECT skip_reason, count(*) FROM files WHERE file_kind = 'skipped' \
         GROUP BY skip_reason ORDER BY skip_reason",
    )?;
    let skipped_by_reason = reason_statement
        .query_map([], |row| {
            Ok(SkipReasonCount {
                reason: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(reason_statement);

    let mut completeness_statement = connection.prepare(
        "SELECT extraction_completeness, count(*) FROM files \
         GROUP BY extraction_completeness ORDER BY extraction_completeness",
    )?;
    let extraction_completeness_counts = completeness_statement
        .query_map([], |row| {
            Ok(CompletenessCount {
                extraction_completeness: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(completeness_statement);

    let skipped_file_count = file_kind_counts
        .iter()
        .find(|entry| entry.kind == "skipped")
        .map_or(0, |entry| entry.count);

    // One `evidence:truncated` row per file (it is unique per file_id/key),
    // so a row count is the file count.
    let evidence_capped_file_count: i64 = connection.query_row(
        "SELECT count(*) FROM evidence WHERE key = 'evidence:truncated'",
        [],
        |row| row.get(0),
    )?;

    let indexed_files_without_evidence: i64 = connection.query_row(
        "SELECT count(*) FROM files f \
         WHERE f.file_kind = 'indexed' \
           AND NOT EXISTS (SELECT 1 FROM evidence e WHERE e.file_id = f.id)",
        [],
        |row| row.get(0),
    )?;

    Ok(CompletenessEnvelope {
        file_kind_counts,
        file_class_counts,
        skipped_file_count,
        skipped_by_reason,
        extraction_completeness_counts,
        index_partial: skipped_file_count > 0,
        evidence_capped_file_count,
        indexed_files_without_evidence,
    })
}
