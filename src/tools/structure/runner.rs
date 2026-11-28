// slugaudit-line-exception: approved-by=split-judge; reason=Single responsibility: executing a structure Tree-sitter query across files within time and match budgets while streaming matches and recording resume boundaries.
//! Budgeted Tree-sitter parsing, `QueryCursor` capture execution, and match
//! streaming over `(path, content)` pairs for `structure`.

use super::match_node::{OutputMode, capture_at, node_to_match, require_captures};
use super::stored::StoredMatch;
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::{LineIndex, ResourceLimits};
use crate::parse::budget::{Parsed, parse_within};
use crate::tools::grammar::run_budgeted_captures;
use crate::tools::paginate::RowSink;
use crate::tools::tool_limits::deadline_after;
use rmcp::ErrorData;
use std::time::Instant;
use tree_sitter::{Parser, Query, QueryCursor, Tree};

#[derive(Default)]
pub(super) struct RunOutput {
    pub(super) files_scanned: usize,
    pub(super) untouched_start: usize,
    pub(super) unparsed_files: Vec<String>,
    pub(super) parse_timed_out_file: Option<String>,
    pub(super) timed_out_resume_file: Option<String>,
    pub(super) budget_resume_after: Option<String>,
    pub(super) query_match_limit_hit: bool,
    pub(super) match_limit_resume_after: Option<String>,
}

/// Runs `query` over `files`, streaming all captured matches into `sink`.
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
        last_completed_file: None,
        output: RunOutput::default(),
    };
    run.scan_files(&mut parser, files)?;
    if run.timed_out {
        return Err(invalid_params_with_code(
            "structure query exceeded its execution time budget (max_structure_execution_time); \
             narrow `query` or `pattern`, target a single `file`, or switch this session to the \
             `audit` profile via `project_control`",
            ErrorCode::StructureBudgetExhausted,
        ));
    }
    Ok(run.output)
}

struct Run<'q, 'w, 's> {
    query: &'q Query,
    mode: &'q OutputMode,
    sink: &'q mut RowSink<'w, 's>,
    deadline: Option<Instant>,
    timed_out: bool,
    cursor: QueryCursor,
    last_completed_file: Option<String>,
    output: RunOutput,
}

fn deadline_spent(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|limit| Instant::now() >= limit)
}

impl Run<'_, '_, '_> {
    fn scan_files(
        &mut self,
        parser: &mut Parser,
        files: &[(String, String)],
    ) -> Result<(), ErrorData> {
        if !files.is_empty() && deadline_spent(self.deadline) {
            self.timed_out = true;
            return Ok(());
        }
        let deadline = self.deadline;
        for (idx, (file_path, content)) in files.iter().enumerate() {
            if deadline_spent(deadline) {
                self.record_timeout(idx, file_path, content.len(), "before parse");
                return Ok(());
            }
            let tree = match parse_within(parser, content, || deadline_spent(deadline)) {
                Parsed::Tree(tree) => tree,
                Parsed::Cancelled => {
                    self.record_timeout(idx, file_path, content.len(), "during parse");
                    return Ok(());
                }
                Parsed::NoTree if self.mode.single_file => {
                    return Err(ErrorData::internal_error(
                        "parser returned no syntax tree",
                        None,
                    ));
                }
                Parsed::NoTree => {
                    self.output.unparsed_files.push(file_path.clone());
                    self.advance_completed(idx, file_path, false)?;
                    continue;
                }
            };
            if !self.scan_tree(idx, file_path, content, &tree)? {
                return Ok(());
            }
            self.advance_completed(idx, file_path, true)?;
        }
        Ok(())
    }

    fn advance_untouched_past(&mut self, idx: usize) {
        // KEEP saturating: idx is a slice index well below usize::MAX
        self.output.untouched_start = idx.saturating_add(1);
    }

    fn record_timeout(
        &mut self,
        idx: usize,
        file_path: &str,
        content_bytes: usize,
        stage: &'static str,
    ) {
        if self.mode.single_file {
            self.timed_out = true;
            return;
        }
        if self.output.files_scanned > 0 {
            tracing::warn!(
                file = %file_path,
                content_bytes,
                stage,
                "structure: execution budget reached after earlier files; resuming before this file",
            );
            self.output.untouched_start = idx;
            self.output.budget_resume_after = self.last_completed_file.take();
            return;
        }
        tracing::warn!(
            file = %file_path,
            content_bytes,
            stage,
            "structure: first file exhausted the execution budget; marking timed out so scan can advance",
        );
        self.advance_untouched_past(idx);
        self.output.timed_out_resume_file = Some(file_path.to_owned());
        if stage != "during query cursor" {
            self.output.parse_timed_out_file = Some(file_path.to_owned());
        }
    }

    fn advance_completed(
        &mut self,
        idx: usize,
        file_path: &str,
        counted: bool,
    ) -> Result<(), ErrorData> {
        self.last_completed_file = Some(file_path.to_owned());
        self.advance_untouched_past(idx);
        if counted {
            self.output.files_scanned =
                self.output.files_scanned.checked_add(1).ok_or_else(|| {
                    ErrorData::internal_error("structure: the scanned-file count overflowed", None)
                })?;
        }
        Ok(())
    }

    fn scan_tree(
        &mut self,
        idx: usize,
        file_path: &str,
        content: &str,
        tree: &Tree,
    ) -> Result<bool, ErrorData> {
        let (file_matches, outcome) = collect_tree_matches(
            &mut self.cursor,
            self.query,
            self.mode.full_text,
            self.deadline,
            file_path,
            content,
            tree,
        )?;
        if outcome.timed_out {
            self.record_timeout(idx, file_path, content.len(), "during query cursor");
            return Ok(false);
        }
        for stored in &file_matches {
            self.sink.push(stored)?;
        }
        if outcome.match_limit_exceeded {
            tracing::warn!(
                file = %file_path,
                emitted = file_matches.len(),
                "structure: QueryCursor exceeded match limit (65,536 active states); marking result partial",
            );
            self.output.query_match_limit_hit = true;
            self.advance_untouched_past(idx);
            if !self.mode.single_file {
                self.output.match_limit_resume_after = Some(file_path.to_owned());
            }
            return Ok(false);
        }
        Ok(true)
    }
}

fn collect_tree_matches(
    cursor: &mut QueryCursor,
    query: &Query,
    full_text: bool,
    deadline: Option<Instant>,
    file_path: &str,
    content: &str,
    tree: &Tree,
) -> Result<(Vec<StoredMatch>, crate::tools::grammar::CursorScanOutcome), ErrorData> {
    let capture_names = query.capture_names();
    let lines = LineIndex::new(content).map_err(|error| {
        ErrorData::internal_error(format!("structure match position: {error}"), None)
    })?;
    let mut file_matches: Vec<StoredMatch> = Vec::new();
    let outcome = run_budgeted_captures(
        cursor,
        query,
        tree.root_node(),
        content.as_bytes(),
        deadline,
        |query_match, capture_index| {
            let capture = capture_at(query_match, capture_index)?;
            let found = node_to_match(
                file_path,
                capture.node,
                content,
                &lines,
                capture_names.get(capture.index as usize).copied(),
                full_text,
            )?;
            file_matches.push(StoredMatch { found });
            Ok::<(), ErrorData>(())
        },
    )?;
    Ok((file_matches, outcome))
}
