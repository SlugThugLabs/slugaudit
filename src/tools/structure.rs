//! The `structure` tool contract: request/response types, input
//! validation, and target loading. Query execution lives in
//! `structure_exec`.

use super::context::{SyncedProject, ensure_synced, with_verified_read};
use super::structure_exec::{OutputMode, run_query};
use crate::model::{ResourceLimits, process_limits};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::Query;

const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_FILTER_BYTES: usize = 8 * 1024;

#[cfg(test)]
pub(super) use super::structure_source::MAX_TEXT_BYTES;
use super::structure_source::{fetch_source, fetch_sources_for_language};

#[derive(Debug, Deserialize, JsonSchema)]
/// A Tree-sitter query to run against one indexed file, or every indexed
/// file of one language.
pub struct StructureRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Project-relative path of a single file to match against.
    /// When omitted, matches all files in the project matching `language`.
    #[serde(default)]
    pub file: Option<String>,
    /// Language grammar to use (e.g. "rust", "python", "typescript").
    /// Required when `file` is omitted; optional if `file` is provided.
    #[serde(default)]
    pub language: Option<String>,
    /// Optional path pattern (e.g. "*auth*", "src/**/*.rs") to filter files.
    #[serde(default)]
    pub pattern: Option<String>,
    /// A tree-sitter S-expression query, e.g. `(function_item name: (identifier) @name)`.
    /// For patterns normalized evidence and `query` can't easily express.
    pub query: String,
    /// Whether to include full matched source text up to MAX_TEXT_BYTES.
    /// Defaults to true for single-file queries, and false (lean 1-line snippet) for multi-file queries.
    #[serde(default)]
    pub full_text: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
/// One capture match mapped back to source coordinates, with the
/// clipping/extraction flags that bound its `text`.
pub struct StructureMatch {
    pub file: String,
    pub capture_name: String,
    pub node_kind: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub text: String,
    pub text_truncated: bool,
    pub extraction_failed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
/// The matches for one structure call, the revision they were read from,
/// and whether the match cap or the deadline cut them short.
pub struct StructureResponse {
    pub revision_id: String,
    pub language: String,
    pub matches: Vec<StructureMatch>,
    pub truncated: bool,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `request.file`
/// isn't indexed with a detected, pack-supported language, the query text
/// is empty/too large or fails to compile, or the parser returns no tree.
pub fn structure(
    request: &Parameters<StructureRequest>,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let limits = resolve_structure_limits(&request.0.path);
    structure_with_limits(request, &limits, sink, manager)
}

fn resolve_structure_limits(path: &str) -> ResourceLimits {
    match crate::project::find_project_root(std::path::Path::new(path)) {
        Ok(root) => crate::project::ProjectConfig::load_or_default(root.as_path())
            .apply_to_limits(*process_limits()),
        Err(_) => *process_limits(),
    }
}

/// Test-only seam: production code always goes through [`structure`] with
/// [`process_limits`]; tests inject a tighter
/// `max_structure_execution_time` to exercise the abort path without
/// waiting out the production-sized budget.
fn structure_with_limits(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
    sink: &dyn crate::progress::ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<StructureResponse>, ErrorData> {
    let request = &request.0;
    validate_request(request, limits)?;

    let synced = ensure_synced(&request.path, sink, manager)?;
    let (target_language, files_to_scan) = load_targets(&synced, request)?;

    let ts_language = tree_sitter_language_pack::get_language(&target_language)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let compiled_query = Query::new(&ts_language, &request.query)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;

    let single_file = request.file.is_some();
    let mode = OutputMode {
        full_text: request.full_text.unwrap_or(single_file),
        single_file,
    };
    let (matches, truncated) =
        run_query(&compiled_query, &ts_language, &files_to_scan, limits, &mode)?;

    Ok(Json(StructureResponse {
        revision_id: synced.revision_id,
        language: target_language,
        matches,
        truncated,
    }))
}

/// Rejects oversized or empty inputs before any sync or parse work.
fn validate_request(request: &StructureRequest, limits: &ResourceLimits) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    for (name, value) in [
        ("file", request.file.as_deref()),
        ("language", request.language.as_deref()),
        ("pattern", request.pattern.as_deref()),
    ] {
        if value.is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_FILTER_BYTES)) {
            return Err(ErrorData::invalid_params(
                format!("{name} exceeds 8192 bytes"),
                None,
            ));
        }
    }
    if request.query.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query must not be empty",
            None,
        ));
    }
    if request.query.len() > limits.max_structure_query_bytes {
        return Err(ErrorData::invalid_params(
            format!(
                "structure query exceeds {} bytes",
                limits.max_structure_query_bytes
            ),
            None,
        ));
    }
    Ok(())
}

/// The grammar to use and the `(path, content)` pairs to scan: the one
/// named file (language detected from the index), or every indexed file of
/// the requested language that matches `pattern`.
fn load_targets(
    synced: &SyncedProject,
    request: &StructureRequest,
) -> Result<(String, Vec<(String, String)>), ErrorData> {
    if let Some(file) = &request.file {
        let (content, detected) = with_verified_read(synced, |tx| fetch_source(tx, file))?;
        return Ok((detected, vec![(file.clone(), content)]));
    }
    let language = request.language.as_deref().ok_or_else(|| {
        ErrorData::invalid_params(
            "structure request without 'file' must specify 'language'",
            None,
        )
    })?;
    let sources = with_verified_read(synced, |tx| {
        fetch_sources_for_language(tx, language, request.pattern.as_deref())
    })?;
    Ok((language.to_owned(), sources))
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "structure_limit_tests.rs"]
mod limit_tests;
