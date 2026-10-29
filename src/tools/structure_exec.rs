// slugaudit-line-exception: approved-by=slugthug; reason=one query execution: the budgeted parse and the budgeted capture cursor share one deadline and one Run state
//! Executing a compiled structure query: parsing each file, running the
//! capture cursor under the match cap and the wall-clock budget, and mapping
//! captured nodes back to source coordinates. The parse runs under the same
//! budget as the cursor, through tree-sitter's parse progress callback.

use super::paginate::{Produced, RowSink};
use super::query_exec::deadline_after;
use super::structure::StructureRequest;
use super::structure::stored::{CutFacts, StoredMatch, StructureExtra};
use super::structure_page::{
    OutputMode, RunOutput, capture_at, completeness_of, extra_of, node_to_match, require_captures,
};
use super::structure_source::{WindowBounds, load_targets};
use crate::model::{LineIndex, ResourceLimits};
use crate::parse::budget::{Parsed, parse_within};
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::ops::ControlFlow;
use std::time::Instant;
use tree_sitter::{
    Parser, Query, QueryCursor, QueryCursorOptions, QueryCursorState, StreamingIterator, Tree,
};

/// Runs `query` over `files`, streaming every match into `sink` in scan order.
/// A spent execution budget is an error, never a silently partial result; the
/// one early stop is a multi-file parse the budget cancelled, reported by
/// `parse_timed_out_file`. Page cuts are applied later, when a stored page is
/// read.
pub(super) fn run_query(
    query: &Query,
    language: &tree_sitter::Language,
    files: &[(String, String)],
    limits: &ResourceLimits,
    mode: &OutputMode,
    sink: &mut RowSink<'_, '_>,
) -> Result<RunOutput, ErrorData> {
    require_captures(query)?;
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let mut run = Run {
        query,
        mode,
        sink,
        deadline: deadline_after(limits.max_structure_execution_time),
        timed_out: false,
        cursor: QueryCursor::new(),
        files_scanned: 0,
        unparsed_files: Vec::new(),
        parse_timed_out_file: None,
    };
    run.scan_files(&mut parser, files)?;
    if run.timed_out {
        return Err(ErrorData::invalid_params(
            "structure query exceeded its execution time budget",
            None,
        ));
    }
    Ok(RunOutput {
        files_scanned: run.files_scanned,
        unparsed_files: run.unparsed_files,
        parse_timed_out_file: run.parse_timed_out_file,
    })
}

/// One query execution's budget and accumulated output.
struct Run<'q, 'w, 's> {
    query: &'q Query,
    mode: &'q OutputMode,
    sink: &'q mut RowSink<'w, 's>,
    /// `None` when the configured budget is too large to be an `Instant`:
    /// no wall-clock deadline (see [`deadline_after`]).
    deadline: Option<Instant>,
    timed_out: bool,
    cursor: QueryCursor,
    /// Files finished before the scan stopped.
    files_scanned: usize,
    unparsed_files: Vec<String>,
    /// Multi-file: the file whose parse the deadline cancelled.
    parse_timed_out_file: Option<String>,
}

impl Run<'_, '_, '_> {
    /// Scans files in order until they run out or the deadline passes. A
    /// parse the deadline cancels ends a multi-file scan with what earlier
    /// files produced, flagged by `parse_timed_out_file`; for a single file
    /// there is nothing to return, so it is the budget error.
    fn scan_files(
        &mut self,
        parser: &mut Parser,
        files: &[(String, String)],
    ) -> Result<(), ErrorData> {
        let deadline = self.deadline;
        for (file_path, content) in files {
            if self.out_of_budget() {
                return Ok(());
            }
            let spent = || deadline.is_some_and(|deadline| Instant::now() >= deadline);
            let tree = match parse_within(parser, content, spent) {
                Parsed::Tree(tree) => tree,
                Parsed::Cancelled if self.mode.single_file => {
                    self.timed_out = true;
                    return Ok(());
                }
                Parsed::Cancelled => {
                    tracing::warn!(
                        file = %file_path,
                        content_bytes = content.len(),
                        "structure: parse cancelled at the execution budget; returning the matches so far",
                    );
                    self.parse_timed_out_file = Some(file_path.clone());
                    return Ok(());
                }
                Parsed::NoTree if self.mode.single_file => {
                    return Err(ErrorData::internal_error(
                        "parser returned no syntax tree",
                        None,
                    ));
                }
                Parsed::NoTree => {
                    self.unparsed_files.push(file_path.clone());
                    self.files_scanned += 1;
                    continue;
                }
            };
            self.scan_tree(file_path, content, &tree)?;
            if self.timed_out {
                break;
            }
            self.files_scanned += 1;
        }
        Ok(())
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

    /// Stores every capture of one file.
    fn scan_tree(&mut self, file_path: &str, content: &str, tree: &Tree) -> Result<(), ErrorData> {
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
        while let Some((query_match, capture_index)) = captures.next() {
            let capture = capture_at(query_match, *capture_index)?;
            let found = node_to_match(
                file_path,
                capture.node,
                content,
                &lines,
                capture_names.get(capture.index as usize).copied(),
                self.mode.full_text,
            );
            self.sink.push(&StoredMatch { found })?;
        }
        drop(captures);
        self.timed_out |= timed_out;
        Ok(())
    }
}

/// Loads the target files, runs the query, and streams every match into the
/// store, all inside the verified read transaction so the window and the
/// scan describe one revision.
pub(super) fn produce(
    tx: &Transaction<'_>,
    rows: &mut RowSink<'_, '_>,
    request: &StructureRequest,
    limits: &ResourceLimits,
    window_bytes: usize,
    cut: CutFacts,
) -> Result<Produced<StructureExtra>, ErrorData> {
    let bounds = WindowBounds {
        window_bytes,
        max_parse_bytes: limits.max_parse_bytes,
    };
    let (target_language, window) = load_targets(tx, request, bounds)?;
    let (ts_language, compiled_query) = compile_query(&target_language, &request.query)?;
    let single_file = request.file.is_some();
    let mode = OutputMode {
        full_text: request.full_text.unwrap_or(single_file),
        single_file,
    };
    let run = run_query(
        &compiled_query,
        &ts_language,
        &window.files,
        limits,
        &mode,
        rows,
    )?;
    let completeness = completeness_of(&window, &run);
    Ok(Produced {
        completeness,
        extra: extra_of(target_language, window, run, cut),
    })
}

/// Compiles the requested grammar and query, mapping failures to contract
/// errors.
fn compile_query(language: &str, text: &str) -> Result<(tree_sitter::Language, Query), ErrorData> {
    let ts_language = tree_sitter_language_pack::get_language(language)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let compiled_query = Query::new(&ts_language, text)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    Ok((ts_language, compiled_query))
}
