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
    pub(super) unparsed_files: Vec<String>,
    pub(super) parse_timed_out_file: Option<String>,
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

impl Run<'_, '_, '_> {
    fn past_deadline(&self) -> bool {
        self.deadline.is_some_and(|limit| Instant::now() >= limit)
    }

    fn scan_files(
        &mut self,
        parser: &mut Parser,
        files: &[(String, String)],
    ) -> Result<(), ErrorData> {
        if !files.is_empty() && self.past_deadline() {
            self.timed_out = true;
            return Ok(());
        }
        let deadline = self.deadline;
        for (file_path, content) in files {
            if self.past_deadline() {
                self.record_timeout(file_path, content.len(), "before parse");
                return Ok(());
            }
            let spent = || deadline.is_some_and(|limit| Instant::now() >= limit);
            let tree = match parse_within(parser, content, spent) {
                Parsed::Tree(tree) => tree,
                Parsed::Cancelled => {
                    self.record_timeout(file_path, content.len(), "during parse");
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
                    self.finish_file(file_path)?;
                    continue;
                }
            };
            if !self.scan_tree(file_path, content, &tree)? {
                return Ok(());
            }
            self.finish_file(file_path)?;
        }
        Ok(())
    }

    fn record_timeout(&mut self, file_path: &str, content_bytes: usize, stage: &'static str) {
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
            self.output.budget_resume_after = self.last_completed_file.take();
            return;
        }
        tracing::warn!(
            file = %file_path,
            content_bytes,
            stage,
            "structure: first file exhausted the execution budget; marking timed out so scan can advance",
        );
        self.output.parse_timed_out_file = Some(file_path.to_owned());
    }

    fn finish_file(&mut self, file_path: &str) -> Result<(), ErrorData> {
        self.last_completed_file = Some(file_path.to_owned());
        self.output.files_scanned = self.output.files_scanned.checked_add(1).ok_or_else(|| {
            ErrorData::internal_error("structure: the scanned-file count overflowed", None)
        })?;
        Ok(())
    }

    fn scan_tree(
        &mut self,
        file_path: &str,
        content: &str,
        tree: &Tree,
    ) -> Result<bool, ErrorData> {
        let capture_names = self.query.capture_names();
        let lines = LineIndex::new(content).map_err(|error| {
            ErrorData::internal_error(format!("structure match position: {error}"), None)
        })?;
        let mut file_matches: Vec<StoredMatch> = Vec::new();
        let full_text = self.mode.full_text;
        let outcome = run_budgeted_captures(
            &mut self.cursor,
            self.query,
            tree.root_node(),
            content.as_bytes(),
            self.deadline,
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
        if outcome.timed_out {
            self.record_timeout(file_path, content.len(), "during query cursor");
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
            if !self.mode.single_file {
                self.output.match_limit_resume_after = Some(file_path.to_owned());
            }
            return Ok(false);
        }
        Ok(true)
    }
}
