// slugaudit-line-exception: approved-by=slugthug; reason=the report is one read-only snapshot contract: request, response schema, and the section queries that fill it stay in one file so field docs, SQL predicates, and the response assembly cannot drift apart; the schema types are the MCP outputSchema
use super::CREDENTIAL_PATH_PREDICATE;
use super::context::{ensure_synced, with_verified_read};
use super::report_completeness::{CompletenessEnvelope, build as build_completeness};
use super::report_edges::{DependencyEdgeTotals, build as build_edge_totals};
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
/// The project to snapshot — any path inside an active project.
pub struct ReportRequest {
    /// Any path inside the active project: its root, or a file/directory under it.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// How many discovered files the last revision attributed to one detected
/// language.
pub struct LanguageCount {
    pub language: String,
    pub file_count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// How many evidence rows the current revision holds for one kind.
pub struct EvidenceKindCount {
    pub kind: String,
    pub count: i64,
}

/// How many import edges fell into each resolution bucket. A project with
/// mostly `External` and `Unresolved` edges and very few `Resolved` ones
/// has an import graph the evidence can't connect — useful context for an
/// AI deciding how much to trust dependency traversal for this project.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportResolutionCount {
    /// One of `Resolved`, `Unresolved`, or `External`.
    pub kind: String,
    pub count: i64,
}

/// A file whose parse failed or was not attempted, with enough detail for
/// an AI to decide whether the failure is worth investigating or just
/// noise (e.g. a `.claude_output.txt` scratch file that should have been
/// excluded, vs. a real source file with a genuine syntax error).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ParserFailureFile {
    pub path: String,
    pub parse_outcome: String,
    /// Set only when the parser returned a hard error (not just syntax
    /// error nodes). `None` for `NotAttempted` and `SyntaxErrors`.
    pub parse_error_reason: Option<String>,
    /// One of `Full`, `Partial`, `ContentOnly`, `Unavailable`. `ContentOnly`
    /// means the file's text *is* indexed but no parser ran over it — the
    /// row is listed for availability reasons, not because extraction
    /// failed, and its content can still be read from `files.content`.
    pub extraction_completeness: String,
    /// One of `Available`, `Unavailable`, `LoadFailed`. Separates
    /// `ContentOnly` rows that have no parser for their language
    /// (`Unavailable`) from ones whose parser could not be loaded
    /// (`LoadFailed`), which a completeness value alone cannot.
    pub parser_availability: String,
}

/// Cap on how many parser-failure files to include in the report. Enough
/// to surface the pattern (e.g. "all failures are scratch files") without
/// letting a project with thousands of broken files bloat the response.
const MAX_PARSER_FAILURE_FILES: usize = 20;
const MAX_PATH_BYTES: usize = 32 * 1024;

/// Automatic project snapshot: what evidence exists, not what's suspicious
/// about it. No score, no risk leads, no safe/unsafe claim.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportResponse {
    pub revision_id: String,
    /// Rows in `files`: every file the last publish discovered, whether or
    /// not it was indexed. Read `completeness.file_kind_counts` to see how
    /// many of those have content (`indexed`), are binary, or were skipped —
    /// this number alone does not mean "files with evidence".
    pub file_count: i64,
    /// Whether every count above is the whole truth: rows by kind, skipped
    /// rows by reason, rows by extraction completeness, and a partial-index
    /// flag. See [`CompletenessEnvelope`].
    pub completeness: CompletenessEnvelope,
    pub languages: Vec<LanguageCount>,
    pub evidence_counts: Vec<EvidenceKindCount>,
    pub parser_failure_count: i64,
    /// Up to `MAX_PARSER_FAILURE_FILES` files with parse failures, so the
    /// AI can see whether failures are concentrated in scratch/temp files
    /// (noise) or in real source files (worth investigating).
    pub parser_failure_files: Vec<ParserFailureFile>,
    /// Uncapped total of files matching the `parser_failure_files` predicate.
    /// Larger than the returned list's length means the list was truncated at
    /// `MAX_PARSER_FAILURE_FILES`; equal means the list is complete.
    pub parser_failure_files_total: i64,
    pub open_finding_count: i64,
    /// How many import edges are Resolved vs Unresolved vs External — a
    /// key asymmetry signal. A project with mostly External/Unresolved
    /// imports has a disconnected import graph that dependency traversal
    /// can't help with.
    pub import_resolution: Vec<ImportResolutionCount>,
    /// The same graph counted in both directions: how many edges leave a
    /// file, how many land on one, and how many files are on each end.
    /// Pure arithmetic over `dependency_edges` — no ranking, no severity,
    /// and no claim about which file matters. Per-file traversal is the
    /// `slugaudit://queries/blast_radius.sql` recipe.
    pub dependency_edges: DependencyEdgeTotals,
    /// Total number of Diagnostic evidence items (e.g. linter or compiler
    /// diagnostics extracted from source files).
    pub diagnostic_count: i64,
    /// Of the `Unresolved` count in `import_resolution`, how many come from
    /// import syntax the resolver couldn't parse at all — `extract_reference`
    /// returned `None` on the raw text (e.g. Go's `import "fmt"`), as
    /// opposed to an import whose syntax was parsed but matched no project
    /// file (e.g. a stdlib module like `kotlin.math.max`). SlugAudit
    /// records both as `Unresolved` edges since it can't tell them apart
    /// at resolution time, but they mean very different things: a high
    /// count here means "this project's import syntax isn't modeled yet,"
    /// not "this project's imports are broken."
    pub unsupported_language_unresolved_count: i64,
    /// How many rows in `files` match common credential-file patterns
    /// (`.env`, `*.pem`, `*.key`, `id_rsa*`, etc. — the same list the
    /// SQL below matches, and the same list `leads`' `credential_files`
    /// rows use). SlugAudit records these paths like any other file —
    /// including when the file was skipped and no content was retained —
    /// so this count makes the AI aware it is handling credential-*looking*
    /// paths even when it cannot read them.
    pub credential_pattern_file_count: i64,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project, or if
/// syncing or querying the project's database fails.
pub fn report(
    request: &Parameters<ReportRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<ReportResponse>, ErrorData> {
    if crate::util::exceeds_input_bytes(&request.0.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    let synced = ensure_synced(&request.0.path, sink, manager)?;
    let revision_id = synced.revision_id.clone();
    let response = with_verified_read(&synced, |tx| {
        build_report(tx, revision_id.clone())
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })?;
    Ok(Json(response))
}

/// Assembles the report from per-section queries. Each section is one
/// small function below; this is the ordering plus the completion log —
/// nothing else. A failure in any section propagates: a report is built
/// from full truth or not at all.
fn build_report(
    connection: &Transaction<'_>,
    revision_id: String,
) -> Result<ReportResponse, rusqlite::Error> {
    let file_count = scalar_count(connection, "SELECT count(*) FROM files")?;
    let completeness = build_completeness(connection)?;
    let languages = grouped_language_counts(connection)?;
    let evidence_counts = grouped_evidence_counts(connection)?;
    let (parser_failure_files_total, parser_failure_files) = parser_failures(connection)?;
    let open_finding_count = scalar_count(
        connection,
        "SELECT count(*) FROM findings WHERE status = 'current'",
    )?;
    let import_resolution = grouped_import_resolution(connection)?;
    // The same edges counted by direction — the zero-decision entry point
    // carries the totals so an agent does not have to write the COUNT to
    // get them; per-file traversal stays in the recipe resource.
    let dependency_edges = build_edge_totals(connection)?;
    // Total diagnostics — linter/compiler diagnostics extracted from source
    // files. Separate from parser failures: a file can parse fine but
    // still have diagnostics.
    let diagnostic_count = scalar_count(
        connection,
        "SELECT count(*) FROM evidence WHERE kind = 'Diagnostic'",
    )?;
    // Of the Unresolved edges, how many come from import syntax the
    // resolver can't parse — see the field doc on
    // `unsupported_language_unresolved_count`. The verdict is cached on
    // each edge at write time (`revision_edges::resolve_and_store` sets
    // `syntax_unmodeled`), so this is a plain SQL count — never a per-edge
    // re-extraction per report call (C11).
    let unsupported_language_unresolved_count = scalar_count(
        connection,
        "SELECT count(*) FROM dependency_edges \
         WHERE resolution_kind = 'Unresolved' AND syntax_unmodeled = 1",
    )?;
    // Credential-pattern files are indexed like any other file (see the
    // field doc); `lower(path)` keeps the match robust to case, and the
    // shared predicate lives in `tools::mod` so `leads` and `report` can
    // never disagree about the pattern list.
    let credential_pattern_file_count = scalar_count(
        connection,
        &format!("SELECT count(*) FROM files WHERE {CREDENTIAL_PATH_PREDICATE}"),
    )?;

    // Counts and the revision id only — never row content — attached to
    // whatever span `run_blocking` (src/server.rs) currently has entered.
    tracing::info!(
        revision_id,
        file_count,
        parser_failure_count = parser_failure_files_total,
        open_finding_count,
        diagnostic_count,
        "report built"
    );
    Ok(ReportResponse {
        revision_id,
        file_count,
        completeness,
        languages,
        evidence_counts,
        parser_failure_count: parser_failure_files_total,
        parser_failure_files,
        parser_failure_files_total,
        open_finding_count,
        import_resolution,
        dependency_edges,
        diagnostic_count,
        unsupported_language_unresolved_count,
        credential_pattern_file_count,
    })
}

/// One `SELECT count(*)` scalar.
fn scalar_count(connection: &Transaction<'_>, sql: &str) -> Result<i64, rusqlite::Error> {
    connection.query_row(sql, [], |row| row.get(0))
}

fn grouped_language_counts(
    connection: &Transaction<'_>,
) -> Result<Vec<LanguageCount>, rusqlite::Error> {
    let mut language_statement = connection.prepare(
        "SELECT language, count(*) FROM files \
         WHERE language IS NOT NULL GROUP BY language ORDER BY language",
    )?;
    let languages = language_statement
        .query_map([], |row| {
            Ok(LanguageCount {
                language: row.get(0)?,
                file_count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(language_statement);
    Ok(languages)
}

fn grouped_evidence_counts(
    connection: &Transaction<'_>,
) -> Result<Vec<EvidenceKindCount>, rusqlite::Error> {
    let mut evidence_statement =
        connection.prepare("SELECT kind, count(*) FROM evidence GROUP BY kind ORDER BY kind")?;
    let evidence_counts = evidence_statement
        .query_map([], |row| {
            Ok(EvidenceKindCount {
                kind: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(evidence_statement);
    Ok(evidence_counts)
}

/// Failures include load/availability problems *and* parses that ran and
/// then failed — both are "the AI cannot trust this file's structural
/// evidence" conditions. Returns the uncapped total for exactly the
/// predicate the capped list selects (so the response can state whether
/// the list is complete without a second scan), and the capped list
/// itself — ordered by path for determinism, so a project with thousands
/// of broken files doesn't bloat the report and the AI can still
/// distinguish noise (scratch files) from real source files.
fn parser_failures(
    connection: &Transaction<'_>,
) -> Result<(i64, Vec<ParserFailureFile>), rusqlite::Error> {
    let total = scalar_count(
        connection,
        "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND (\
            parser_availability != 'Available' OR parse_outcome = 'Failed'\
         )",
    )?;
    let mut failure_stmt = connection.prepare(
        "SELECT path, parse_outcome, parse_error_reason, extraction_completeness, \
                parser_availability \
         FROM files \
         WHERE file_kind = 'indexed' \
           AND (parser_availability != 'Available' OR parse_outcome = 'Failed') \
         ORDER BY path \
         LIMIT ?1",
    )?;
    let files = failure_stmt
        .query_map([MAX_PARSER_FAILURE_FILES], |row| {
            Ok(ParserFailureFile {
                path: row.get(0)?,
                parse_outcome: row.get(1)?,
                parse_error_reason: row.get(2)?,
                extraction_completeness: row.get(3)?,
                parser_availability: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(failure_stmt);
    Ok((total, files))
}

/// Import resolution breakdown — the key asymmetry signal. A project
/// where most imports are External or Unresolved has a disconnected
/// import graph; dependency traversal won't help much there.
fn grouped_import_resolution(
    connection: &Transaction<'_>,
) -> Result<Vec<ImportResolutionCount>, rusqlite::Error> {
    let mut import_resolution_stmt = connection.prepare(
        "SELECT resolution_kind, count(*) \
         FROM dependency_edges \
         GROUP BY resolution_kind ORDER BY resolution_kind",
    )?;
    let import_resolution = import_resolution_stmt
        .query_map([], |row| {
            Ok(ImportResolutionCount {
                kind: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(import_resolution_stmt);
    Ok(import_resolution)
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
