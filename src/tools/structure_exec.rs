//! Executing a compiled structure query: parsing each file, running the
//! capture cursor under the match cap and the wall-clock budget, and mapping
//! captured nodes back to source coordinates.

use super::structure::StructureMatch;
use super::structure_source::{MAX_SNIPPET_BYTES, truncate_snippet, truncate_text};
use crate::model::{ResourceLimits, char_column, saturating_u32};
use rmcp::ErrorData;
use std::ops::ControlFlow;
use std::time::Instant;
use tree_sitter::{
    Parser, Query, QueryCursor, QueryCursorOptions, QueryCursorState, StreamingIterator, Tree,
};

/// How a run reports its matches.
pub(super) struct OutputMode {
    /// Full matched text (up to `MAX_TEXT_BYTES`) instead of a 1-line snippet.
    pub(super) full_text: bool,
    /// A single-file run fails on an unparseable file; a multi-file run skips it.
    pub(super) single_file: bool,
}

/// Runs `query` over `files`, returning the matches and whether the match
/// cap cut them short. A spent execution budget is an error, never a
/// silently partial result.
pub(super) fn run_query(
    query: &Query,
    language: &tree_sitter::Language,
    files: &[(String, String)],
    limits: &ResourceLimits,
    mode: &OutputMode,
) -> Result<(Vec<StructureMatch>, bool), ErrorData> {
    require_captures(query)?;
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let mut run = Run {
        query,
        mode,
        max_matches: limits.max_structure_matches,
        deadline: Instant::now() + limits.max_structure_execution_time,
        timed_out: false,
        cursor: QueryCursor::new(),
        matches: Vec::new(),
    };
    let truncated = run.scan_files(&mut parser, files)?;
    if run.timed_out {
        return Err(ErrorData::invalid_params(
            "structure query exceeded its execution time budget",
            None,
        ));
    }
    Ok((run.matches, truncated))
}

/// A bare pattern with no `@capture` can never return a node; reject it
/// with guidance instead of answering an empty, misleading success.
fn require_captures(query: &Query) -> Result<(), ErrorData> {
    if query.capture_names().is_empty() {
        return Err(ErrorData::invalid_params(
            "structure query declares no @capture, so it can never return any node; add "
                .to_owned()
                + "one, e.g. `(function_item name: (identifier) @name)` for Rust or "
                + "`(function_definition) @node` for Python/JS",
            None,
        ));
    }
    Ok(())
}

/// One query execution's budget and accumulated output.
struct Run<'q> {
    query: &'q Query,
    mode: &'q OutputMode,
    max_matches: usize,
    deadline: Instant,
    timed_out: bool,
    cursor: QueryCursor,
    matches: Vec<StructureMatch>,
}

impl Run<'_> {
    /// Scans files in order until they run out, the cap fills, or the
    /// deadline passes. Returns whether the result was cut short.
    fn scan_files(
        &mut self,
        parser: &mut Parser,
        files: &[(String, String)],
    ) -> Result<bool, ErrorData> {
        for (file_path, content) in files {
            if self.out_of_budget() {
                return Ok(true);
            }
            let Some(tree) = parser.parse(content, None) else {
                if self.mode.single_file {
                    return Err(ErrorData::internal_error(
                        "parser returned no syntax tree",
                        None,
                    ));
                }
                continue;
            };
            if self.scan_tree(file_path, content, &tree) {
                return Ok(true);
            }
            if self.timed_out {
                break;
            }
        }
        Ok(false)
    }

    /// Whether another file may be scanned; records a passed deadline.
    fn out_of_budget(&mut self) -> bool {
        if Instant::now() >= self.deadline {
            self.timed_out = true;
            return true;
        }
        self.matches.len() >= self.max_matches
    }

    /// Collects `tree`'s captures up to the cap. Returns whether captures
    /// remained past it.
    fn scan_tree(&mut self, file_path: &str, content: &str, tree: &Tree) -> bool {
        let deadline = self.deadline;
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
        let capture_names = self.query.capture_names();
        let mut captures = self.cursor.captures_with_options(
            self.query,
            tree.root_node(),
            content.as_bytes(),
            options,
        );
        while self.matches.len() < self.max_matches
            && let Some((query_match, capture_index)) = captures.next()
        {
            let capture = query_match.captures[*capture_index];
            self.matches.push(node_to_match(
                file_path,
                capture.node,
                content,
                capture_names.get(capture.index as usize).copied(),
                self.mode.full_text,
            ));
        }
        let more_remain = captures.next().is_some();
        drop(captures);
        self.timed_out |= timed_out;
        more_remain
    }
}

fn node_to_match(
    file: &str,
    node: tree_sitter::Node<'_>,
    content: &str,
    capture_name: Option<&str>,
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
    StructureMatch {
        file: file.to_owned(),
        capture_name: capture_name.unwrap_or_default().to_owned(),
        node_kind: node.kind().to_owned(),
        start_byte: node.start_byte() as u64,
        end_byte: node.end_byte() as u64,
        start_line: saturating_u32(node.start_position().row),
        start_column: char_column(content, node.start_byte()),
        end_line: saturating_u32(node.end_position().row),
        end_column: char_column(content, node.end_byte()),
        text,
        text_truncated,
        extraction_failed: text_extraction_failed || capture_name.is_none(),
    }
}
