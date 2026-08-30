// slugaudit-line-exception: approved-by=agent; reason=the structure tool's query handling (input validation, fetch_source, the capture-budgeted execution with its progress-callback abort, and node-to-match conversion) is one cohesive concern; splitting the capture execution away from its only consumer would fragment the contract that bare no-capture queries are rejected with guidance

use super::context::{ensure_synced, with_verified_read};
use crate::model::{ResourceLimits, char_column, process_limits, saturating_u32};
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ops::ControlFlow;
use std::time::Instant;
use tree_sitter::{
    Parser, Query, QueryCursor, QueryCursorOptions, QueryCursorState, StreamingIterator,
};

const MAX_TEXT_BYTES: usize = 2_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StructureRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Project-relative path of the file to match against.
    pub file: String,
    /// A tree-sitter S-expression query, e.g. `(function_item name: (identifier) @name)`.
    /// For patterns normalized evidence and `query` can't easily express.
    pub query: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StructureMatch {
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
    /// True when `text`/`capture_name` couldn't be extracted (non-UTF8 node
    /// span or out-of-range capture index) and were replaced with "".
    pub extraction_failed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
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
    structure_with_limits(request, process_limits(), sink, manager)
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
    let StructureRequest { path, file, query } = &request.0;
    if query.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query must not be empty",
            None,
        ));
    }
    if query.len() > limits.max_structure_query_bytes {
        return Err(ErrorData::invalid_params(
            format!(
                "structure query exceeds {} bytes",
                limits.max_structure_query_bytes
            ),
            None,
        ));
    }

    let synced = ensure_synced(path, sink, manager)?;
    let revision_id = synced.revision_id.clone();
    let (content, language) = with_verified_read(&synced, |tx| fetch_source(tx, file))?;

    let ts_language = tree_sitter_language_pack::get_language(&language)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let compiled_query = Query::new(&ts_language, query)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;

    let mut parser = Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let tree = parser
        .parse(&content, None)
        .ok_or_else(|| ErrorData::internal_error("parser returned no syntax tree", None))?;

    let (matches, truncated) = run_query(&compiled_query, &tree, &content, limits)?;

    Ok(Json(StructureResponse {
        revision_id,
        language: language.to_owned(),
        matches,
        truncated,
    }))
}

fn fetch_source(
    connection: &rusqlite::Connection,
    file: &str,
) -> Result<(String, String), ErrorData> {
    let (content, language): (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT content, language FROM files WHERE path = ?1",
            [file],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| ErrorData::invalid_params(format!("{file}: {error}"), None))?;
    let content = content.ok_or_else(|| {
        ErrorData::invalid_params(format!("{file} has no indexed source content"), None)
    })?;
    let language = language.ok_or_else(|| {
        ErrorData::invalid_params(format!("{file} has no detected language"), None)
    })?;
    Ok((content, language))
}

/// Runs the compiled query under a native Tree-sitter execution-time budget
/// (aborting pathological patterns mid-query via `QueryCursorOptions::
/// progress_callback`).
///
/// This tool reports *captures*, not raw matches: the tree-sitter API only
/// exposes a matched node when bound to a `@name` capture. A bare pattern
/// like `(function_definition)` compiles and matches but has zero captures,
/// so it would silently return empty — the footgun that once made an agent
/// believe `structure` couldn't parse Python. Queries declaring no captures
/// are therefore rejected with an actionable add-a-capture message.
///
/// # Errors
///
/// Returns an error if the query declares no captures, or if the execution
/// time budget is exceeded.
fn run_query(
    compiled_query: &Query,
    tree: &tree_sitter::Tree,
    content: &str,
    limits: &ResourceLimits,
) -> Result<(Vec<StructureMatch>, bool), ErrorData> {
    let capture_names = compiled_query.capture_names();
    if capture_names.is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query declares no @capture, so it can never return any node; add "
                .to_owned()
                + "one, e.g. `(function_item name: (identifier) @name)` for Rust or "
                + "`(function_definition) @node` for Python/JS",
            None,
        ));
    }

    let mut cursor = QueryCursor::new();
    let deadline = Instant::now() + limits.max_structure_execution_time;
    let mut timed_out = false;
    let mut check_deadline = |_state: &QueryCursorState| -> ControlFlow<()> {
        if Instant::now() >= deadline {
            timed_out = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = QueryCursorOptions::new().progress_callback(&mut check_deadline);

    let mut matches = Vec::new();
    let mut captures = cursor.captures_with_options(
        compiled_query,
        tree.root_node(),
        content.as_bytes(),
        options,
    );
    while matches.len() < limits.max_structure_matches
        && let Some((query_match, capture_index)) = captures.next()
    {
        let capture = query_match.captures[*capture_index];
        let node = capture.node;
        matches.push(node_to_match(
            node,
            content,
            capture_names,
            capture.index as usize,
        ));
    }
    let truncated = matches.len() >= limits.max_structure_matches && captures.next().is_some();
    drop(captures);

    if timed_out {
        return Err(ErrorData::invalid_params(
            "structure query exceeded its execution time budget",
            None,
        ));
    }
    Ok((matches, truncated))
}

/// Converts a single Tree-sitter node (bound to a capture) into a
/// [`StructureMatch`], looking up the capture's name by its index.
fn node_to_match(
    node: tree_sitter::Node<'_>,
    content: &str,
    capture_names: &[&str],
    capture_ix: usize,
) -> StructureMatch {
    let (node_text, text_extraction_failed) = match node.utf8_text(content.as_bytes()) {
        Ok(text) => (text, false),
        Err(_) => ("", true),
    };
    let (text, text_truncated) = truncate_text(node_text);
    let (capture_name, capture_name_missing) = match capture_names.get(capture_ix) {
        Some(name) => ((*name).to_owned(), false),
        None => (String::new(), true),
    };
    StructureMatch {
        capture_name,
        node_kind: node.kind().to_owned(),
        start_byte: node.start_byte() as u64,
        end_byte: node.end_byte() as u64,
        start_line: saturating_u32(node.start_position().row),
        start_column: char_column(content, node.start_byte()),
        end_line: saturating_u32(node.end_position().row),
        end_column: char_column(content, node.end_byte()),
        text,
        text_truncated,
        extraction_failed: text_extraction_failed || capture_name_missing,
    }
}

fn truncate_text(text: &str) -> (String, bool) {
    if text.len() <= MAX_TEXT_BYTES {
        return (text.to_owned(), false);
    }
    let mut end = MAX_TEXT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "structure_limit_tests.rs"]
mod limit_tests;
