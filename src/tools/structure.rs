// slugaudit-line-exception: approved-by=slugthug; reason=the structure tool's query handling (input validation, single- and multi-file dispatch, capture execution with progress abort, and node conversion) is one cohesive concern; splitting would fragment the contract that bare no-capture queries are rejected with guidance

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

#[cfg(test)]
pub(super) use super::structure_source::MAX_TEXT_BYTES;
use super::structure_source::{
    MAX_SNIPPET_BYTES, fetch_source, fetch_sources_for_language, truncate_snippet, truncate_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
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
    let StructureRequest {
        path,
        file,
        language,
        pattern,
        query,
        full_text,
    } = &request.0;
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

    let (target_language, files_to_scan) = match file {
        Some(f) => {
            let (content, detected) = with_verified_read(&synced, |tx| fetch_source(tx, f))?;
            (detected, vec![(f.clone(), content)])
        }
        None => {
            let lang = language.as_deref().ok_or_else(|| {
                ErrorData::invalid_params(
                    "structure request without 'file' must specify 'language'",
                    None,
                )
            })?;
            let sources = with_verified_read(&synced, |tx| {
                fetch_sources_for_language(tx, lang, pattern.as_deref())
            })?;
            (lang.to_owned(), sources)
        }
    };

    let ts_language = tree_sitter_language_pack::get_language(&target_language)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let compiled_query = Query::new(&ts_language, query)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;

    let is_single_file = file.is_some();
    let want_full_text = full_text.unwrap_or(is_single_file);

    let (matches, truncated) = run_query(
        &compiled_query,
        &ts_language,
        &files_to_scan,
        limits,
        want_full_text,
        is_single_file,
    )?;

    Ok(Json(StructureResponse {
        revision_id,
        language: target_language,
        matches,
        truncated,
    }))
}

fn run_query(
    compiled_query: &Query,
    ts_language: &tree_sitter::Language,
    files: &[(String, String)],
    limits: &ResourceLimits,
    want_full_text: bool,
    is_single_file: bool,
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

    let mut parser = Parser::new();
    parser
        .set_language(ts_language)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    let deadline = Instant::now() + limits.max_structure_execution_time;
    let mut timed_out = false;
    let mut cursor = QueryCursor::new();
    let mut matches = Vec::new();
    let mut truncated = false;

    for (file_path, content) in files {
        if matches.len() >= limits.max_structure_matches || Instant::now() >= deadline {
            if Instant::now() >= deadline {
                timed_out = true;
            }
            truncated = true;
            break;
        }

        let Some(tree) = parser.parse(content, None) else {
            if is_single_file {
                return Err(ErrorData::internal_error(
                    "parser returned no syntax tree",
                    None,
                ));
            }
            continue;
        };

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
        while matches.len() < limits.max_structure_matches
            && let Some((query_match, capture_index)) = captures.next()
        {
            let capture = query_match.captures[*capture_index];
            matches.push(node_to_match(
                file_path,
                capture.node,
                content,
                capture_names,
                capture.index as usize,
                want_full_text,
            ));
        }
        if captures.next().is_some() {
            truncated = true;
            break;
        }
        if timed_out {
            break;
        }
    }

    if timed_out {
        return Err(ErrorData::invalid_params(
            "structure query exceeded its execution time budget",
            None,
        ));
    }
    Ok((matches, truncated))
}

fn node_to_match(
    file: &str,
    node: tree_sitter::Node<'_>,
    content: &str,
    capture_names: &[&str],
    capture_ix: usize,
    want_full_text: bool,
) -> StructureMatch {
    let (node_text, text_extraction_failed) = match node.utf8_text(content.as_bytes()) {
        Ok(text) => (text, false),
        Err(_) => ("", true),
    };
    let (text, text_truncated) = if want_full_text {
        truncate_text(node_text)
    } else {
        truncate_snippet(node_text, MAX_SNIPPET_BYTES)
    };
    let (capture_name, capture_name_missing) = match capture_names.get(capture_ix) {
        Some(name) => ((*name).to_owned(), false),
        None => (String::new(), true),
    };
    StructureMatch {
        file: file.to_owned(),
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

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "structure_limit_tests.rs"]
mod limit_tests;
