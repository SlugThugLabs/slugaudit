use super::context::{ensure_synced, with_verified_read};
use crate::model::{ResourceLimits, saturating_u32};
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
) -> Result<Json<StructureResponse>, ErrorData> {
    structure_with_limits(request, &ResourceLimits::default())
}

/// Test-only seam: production code always goes through [`structure`] with
/// [`ResourceLimits::default`]; tests inject a tighter
/// `max_structure_execution_time` to exercise the abort path without
/// waiting out the production-sized budget.
fn structure_with_limits(
    request: &Parameters<StructureRequest>,
    limits: &ResourceLimits,
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

    let synced = ensure_synced(path)?;
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
        language,
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

/// Runs the compiled query under a native Tree-sitter execution-time budget:
/// `QueryCursorOptions::progress_callback` fires periodically *during*
/// matching (checked roughly every 100 internal operations by the C core),
/// so a pathological pattern is aborted mid-query rather than only after it
/// returns, unlike a wall-clock check wrapped around the whole call.
///
/// # Errors
///
/// Returns an error if `limits.max_structure_execution_time` is exceeded.
fn run_query(
    compiled_query: &Query,
    tree: &tree_sitter::Tree,
    content: &str,
    limits: &ResourceLimits,
) -> Result<(Vec<StructureMatch>, bool), ErrorData> {
    let capture_names = compiled_query.capture_names();
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
    let mut captures = cursor.captures_with_options(
        compiled_query,
        tree.root_node(),
        content.as_bytes(),
        options,
    );

    let mut matches = Vec::new();
    while matches.len() < limits.max_structure_matches
        && let Some((query_match, capture_index)) = captures.next()
    {
        let capture = query_match.captures[*capture_index];
        let node = capture.node;
        let node_text = node.utf8_text(content.as_bytes()).unwrap_or_default();
        let (text, text_truncated) = truncate_text(node_text);
        matches.push(StructureMatch {
            capture_name: capture_names
                .get(capture.index as usize)
                .map(|name| (*name).to_owned())
                .unwrap_or_default(),
            node_kind: node.kind().to_owned(),
            start_byte: node.start_byte() as u64,
            end_byte: node.end_byte() as u64,
            start_line: saturating_u32(node.start_position().row),
            start_column: saturating_u32(node.start_position().column),
            end_line: saturating_u32(node.end_position().row),
            end_column: saturating_u32(node.end_position().column),
            text,
            text_truncated,
        });
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
