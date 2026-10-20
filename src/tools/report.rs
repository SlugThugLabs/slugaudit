use super::context::{ensure_synced, with_verified_read};
use super::report_completeness::CompletenessEnvelope;
use super::report_edges::DependencyEdgeTotals;
#[cfg(test)]
use super::report_sections::MAX_PARSER_FAILURE_FILES;
use super::report_sections::build_report;
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
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

const MAX_PATH_BYTES: usize = 32 * 1024;

/// Automatic project snapshot: what evidence exists, not what's suspicious
/// about it. No score, no risk leads, no safe/unsafe claim.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportResponse {
    pub revision_id: String,
    /// Rows in `files`: every discovered file whose project-relative path is
    /// representable as UTF-8, whether or not it was indexed. A discovered
    /// path that is not valid UTF-8 cannot have a row and is counted by
    /// `completeness.unrepresentable_path_count` instead. Read
    /// `completeness.file_kind_counts` to see how many of these rows have
    /// content (`indexed`), are binary, or were skipped — this number alone
    /// does not mean "files with evidence".
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

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
