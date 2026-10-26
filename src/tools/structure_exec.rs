// slugaudit-line-exception: approved-by=slugthug; reason=one query execution: the budgeted parse and the budgeted capture cursor share one deadline and one Run state
//! Executing a compiled structure query: parsing each file, running the
//! capture cursor under the match cap and the wall-clock budget, and mapping
//! captured nodes back to source coordinates. The parse runs under the same
//! budget as the cursor, through tree-sitter's parse progress callback.

use super::query_exec::deadline_after;
use super::structure::StructureMatch;
use super::structure_page::{
    OutputMode, Paging, RunOutput, capture_at, capture_to_match, require_captures,
};
use crate::model::{LineIndex, ResourceLimits};
use crate::parse::budget::{Parsed, parse_within};
use rmcp::ErrorData;
use std::ops::ControlFlow;
use std::time::Instant;
use tree_sitter::{
    Parser, Query, QueryCursor, QueryCursorOptions, QueryCursorState, StreamingIterator, Tree,
};

/// Runs `query` over `files`, returning one page of matches. `truncated` is
/// true only when a further match was actually seen past the page. A spent
/// execution budget is an error, never a silently partial result.
pub(super) fn run_query(
    query: &Query,
    language: &tree_sitter::Language,
    files: &[(String, String)],
    limits: &ResourceLimits,
    mode: &OutputMode,
    paging: &Paging,
) -> Result<RunOutput, ErrorData> {
    require_captures(query)?;
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let mut run = Run {
        query,
        mode,
        max_matches: limits.max_structure_matches,
        deadline: deadline_after(limits.max_structure_execution_time),
        timed_out: false,
        cursor: QueryCursor::new(),
        matches: Vec::new(),
        skip: paging.offset,
        page_bytes: paging.page_bytes,
        matches_bytes: 0,
        unparsed_files: Vec::new(),
        parse_timed_out_file: None,
    };
    let truncated = run.scan_files(&mut parser, files)?;
    if run.timed_out {
        return Err(ErrorData::invalid_params(
            "structure query exceeded its execution time budget",
            None,
        ));
    }
    Ok(RunOutput {
        matches: run.matches,
        truncated,
        matches_bytes: run.matches_bytes,
        unparsed_files: run.unparsed_files,
        parse_timed_out_file: run.parse_timed_out_file,
    })
}

/// One query execution's budget and accumulated output.
struct Run<'q> {
    query: &'q Query,
    mode: &'q OutputMode,
    max_matches: usize,
    /// `None` when the configured budget is too large to be an `Instant`:
    /// no wall-clock deadline (see [`deadline_after`]).
    deadline: Option<Instant>,
    timed_out: bool,
    cursor: QueryCursor,
    matches: Vec<StructureMatch>,
    skip: usize,
    page_bytes: usize,
    matches_bytes: usize,
    unparsed_files: Vec<String>,
    /// Multi-file: the file whose parse the deadline cancelled.
    parse_timed_out_file: Option<String>,
}

impl Run<'_> {
    /// Scans files in order until they run out, a further match is seen past
    /// a full page, or the deadline passes. Returns whether such a further
    /// match exists. A parse the deadline cancels ends a multi-file scan with
    /// what earlier files produced, flagged by `parse_timed_out_file`; for a
    /// single file there is nothing to return, so it is the budget error.
    fn scan_files(
        &mut self,
        parser: &mut Parser,
        files: &[(String, String)],
    ) -> Result<bool, ErrorData> {
        let deadline = self.deadline;
        for (file_path, content) in files {
            if self.out_of_budget() {
                return Ok(true);
            }
            let spent = || deadline.is_some_and(|deadline| Instant::now() >= deadline);
            let tree = match parse_within(parser, content, spent) {
                Parsed::Tree(tree) => tree,
                Parsed::Cancelled if self.mode.single_file => {
                    self.timed_out = true;
                    return Ok(false);
                }
                Parsed::Cancelled => {
                    tracing::warn!(
                        file_id = %crate::util::redacted_path(std::path::Path::new(file_path)),
                        content_bytes = content.len(),
                        "structure: parse cancelled at the execution budget; returning the matches so far",
                    );
                    self.parse_timed_out_file = Some(file_path.clone());
                    return Ok(false);
                }
                Parsed::NoTree if self.mode.single_file => {
                    return Err(ErrorData::internal_error(
                        "parser returned no syntax tree",
                        None,
                    ));
                }
                Parsed::NoTree => {
                    self.unparsed_files.push(file_path.clone());
                    continue;
                }
            };
            if self.scan_tree(file_path, content, &tree)? {
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
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.timed_out = true;
            return true;
        }
        false
    }

    /// Skips `offset` captures, then collects until the match cap or the page
    /// byte budget is full. Returns whether a capture was seen past the page.
    fn scan_tree(
        &mut self,
        file_path: &str,
        content: &str,
        tree: &Tree,
    ) -> Result<bool, ErrorData> {
        let deadline = self.deadline;
        let mut timed_out = false;
        let mut check_deadline = |_state: &QueryCursorState| -> ControlFlow<()> {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
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
        // One index per file: columns are O(log lines), not line length.
        let lines = LineIndex::new(content);
        let mut more_remain = false;
        while let Some((query_match, capture_index)) = captures.next() {
            if self.skip > 0 {
                self.skip -= 1;
                continue;
            }
            if self.matches.len() >= self.max_matches {
                more_remain = true;
                break;
            }
            let capture = capture_at(query_match, *capture_index)?;
            let (found, found_bytes) = capture_to_match(
                file_path,
                content,
                &lines,
                capture,
                capture_names.get(capture.index as usize).copied(),
                self.mode.full_text,
            )?;
            if !self.matches.is_empty()
                && self.matches_bytes.saturating_add(found_bytes) > self.page_bytes
            {
                more_remain = true;
                break;
            }
            self.matches_bytes = self.matches_bytes.saturating_add(found_bytes);
            self.matches.push(found);
        }
        drop(captures);
        self.timed_out |= timed_out;
        Ok(more_remain)
    }
}
