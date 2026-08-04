use super::context::{SyncRecencyCache, ensure_synced, with_verified_read};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReportRequest {
    /// Any path inside the active project: its root, or a file/directory under it.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LanguageCount {
    pub language: String,
    pub file_count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
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
}

/// Cap on how many parser-failure files to include in the report. Enough
/// to surface the pattern (e.g. "all failures are scratch files") without
/// letting a project with thousands of broken files bloat the response.
const MAX_PARSER_FAILURE_FILES: usize = 20;

/// Automatic project snapshot: what evidence exists, not what's suspicious
/// about it. No score, no risk leads, no safe/unsafe claim.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportResponse {
    pub revision_id: String,
    pub file_count: i64,
    pub languages: Vec<LanguageCount>,
    pub evidence_counts: Vec<EvidenceKindCount>,
    pub parser_failure_count: i64,
    /// Up to `MAX_PARSER_FAILURE_FILES` files with parse failures, so the
    /// AI can see whether failures are concentrated in scratch/temp files
    /// (noise) or in real source files (worth investigating).
    pub parser_failure_files: Vec<ParserFailureFile>,
    pub open_finding_count: i64,
    /// How many import edges are Resolved vs Unresolved vs External — a
    /// key asymmetry signal. A project with mostly External/Unresolved
    /// imports has a disconnected import graph that dependency traversal
    /// can't help with.
    pub import_resolution: Vec<ImportResolutionCount>,
    /// Total number of Diagnostic evidence items (e.g. linter or compiler
    /// diagnostics extracted from source files).
    pub diagnostic_count: i64,
    /// Of the `Unresolved` count in `import_resolution`, how many are from
    /// files whose language import resolution doesn't model at all (as
    /// opposed to a genuinely broken/missing import in a supported
    /// language). SlugAudit records both as `Unresolved` edges since it
    /// can't tell them apart at resolution time, but they mean very
    /// different things: a high count here means "we can't see this
    /// language's imports yet," not "this project's imports are broken."
    pub unsupported_language_unresolved_count: i64,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project, or if
/// syncing or querying the project's database fails.
pub fn report(
    request: &Parameters<ReportRequest>,
    cache: &SyncRecencyCache,
) -> Result<Json<ReportResponse>, ErrorData> {
    let synced = ensure_synced(&request.0.path, cache)?;
    let revision_id = synced.revision_id.clone();
    let response = with_verified_read(&synced, |tx| {
        build_report(tx, revision_id.clone())
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })?;
    Ok(Json(response))
}

fn build_report(
    connection: &Transaction<'_>,
    revision_id: String,
) -> Result<ReportResponse, rusqlite::Error> {
    let file_count: i64 =
        connection.query_row("SELECT count(*) FROM files", [], |row| row.get(0))?;

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

    // Failures include load/availability problems *and* parses that ran
    // and then failed — both are "the AI cannot trust this file's
    // structural evidence" conditions.
    let parser_failure_count: i64 = connection.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND (\
            parser_availability != 'Available' OR parse_outcome = 'Failed'\
         )",
        [],
        |row| row.get(0),
    )?;

    // Individual files with parse failures, capped so a project with
    // thousands of broken files doesn't bloat the report. Ordered by path
    // for determinism. The AI uses this to distinguish noise (scratch
    // files, non-source files) from real source files worth investigating.
    let mut failure_stmt = connection.prepare(
        "SELECT path, parse_outcome, parse_error_reason \
         FROM files \
         WHERE file_kind = 'indexed' \
           AND (parser_availability != 'Available' OR parse_outcome = 'Failed') \
         ORDER BY path \
         LIMIT ?1",
    )?;
    let parser_failure_files = failure_stmt
        .query_map([MAX_PARSER_FAILURE_FILES], |row| {
            Ok(ParserFailureFile {
                path: row.get(0)?,
                parse_outcome: row.get(1)?,
                parse_error_reason: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(failure_stmt);

    let open_finding_count: i64 = connection.query_row(
        "SELECT count(*) FROM findings WHERE status = 'current'",
        [],
        |row| row.get(0),
    )?;

    // Import resolution breakdown — the key asymmetry signal. A project
    // where most imports are External or Unresolved has a disconnected
    // import graph; dependency traversal won't help much there.
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

    // Total diagnostics — linter/compiler diagnostics extracted from source
    // files. Separate from parser failures: a file can parse fine but
    // still have diagnostics.
    let diagnostic_count: i64 = connection.query_row(
        "SELECT count(*) FROM evidence WHERE kind = 'Diagnostic'",
        [],
        |row| row.get(0),
    )?;

    // Of the Unresolved edges, how many come from a file whose language
    // import resolution doesn't model at all. Computed in Rust rather than
    // SQL so the language list has one source of truth
    // (`graph::is_supported_language`), not a second copy embedded in a
    // query string that could drift out of sync with it.
    let mut unresolved_language_stmt = connection.prepare(
        "SELECT f.language FROM dependency_edges de \
         JOIN files f ON f.id = de.from_file_id \
         WHERE de.resolution_kind = 'Unresolved'",
    )?;
    let unresolved_languages: Vec<Option<String>> = unresolved_language_stmt
        .query_map([], |row| row.get::<_, Option<String>>(0))?
        .collect::<Result<_, _>>()?;
    let unsupported_language_unresolved_count = unresolved_languages
        .into_iter()
        .flatten()
        .filter(|language| !crate::graph::is_supported_language(language))
        .count() as i64;
    drop(unresolved_language_stmt);

    // Counts and the revision id only — never row content — attached to
    // whatever span `run_blocking` (src/server.rs) currently has entered.
    tracing::info!(
        revision_id,
        file_count,
        parser_failure_count,
        open_finding_count,
        diagnostic_count,
        "report built"
    );
    Ok(ReportResponse {
        revision_id,
        file_count,
        languages,
        evidence_counts,
        parser_failure_count,
        parser_failure_files,
        open_finding_count,
        import_resolution,
        diagnostic_count,
        unsupported_language_unresolved_count,
    })
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
