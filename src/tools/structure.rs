use super::context::{ensure_synced, with_verified_read};
use crate::model::{ResourceLimits, saturating_u32};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

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
    let StructureRequest { path, file, query } = &request.0;
    let limits = ResourceLimits::default();
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

    let (matches, truncated) = run_query(
        &compiled_query,
        &tree,
        &content,
        limits.max_structure_matches,
    );

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

fn run_query(
    compiled_query: &Query,
    tree: &tree_sitter::Tree,
    content: &str,
    max_matches: usize,
) -> (Vec<StructureMatch>, bool) {
    let capture_names = compiled_query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(compiled_query, tree.root_node(), content.as_bytes());

    let mut matches = Vec::new();
    while matches.len() < max_matches
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
    let truncated = matches.len() >= max_matches && captures.next().is_some();
    (matches, truncated)
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
