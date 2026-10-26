// slugaudit-line-exception: approved-by=slugthug; reason=one registered-mechanic scan: the budgeted parse and the budgeted capture cursor share one deadline and one file stream
//! Running registered mechanics: each entry's compiled Tree-sitter query over
//! the stored content of the indexed files of its language, under one
//! wall-clock budget per call.
//!
//! The files read are exactly the ones a built-in evidence mechanic reads —
//! indexed, with a grammar that ran at sync (`leads_covers::PARSED_FILES`) —
//! so a file the grammar could not parse is already counted by
//! `files_not_parsed` and never silently skipped here. Files stream one at a
//! time in path order and each is parsed once for every entry of its
//! language; a capture is one hit, de-duplicated per node, so a node two
//! patterns of one query both capture is counted once.
//!
//! A spent budget fails the call: a partial count would read as a complete
//! one. Only the entries a call needs are scanned (`leads` naming a built-in
//! scans none), so one slow entry cannot fail calls that never asked for it,
//! and the failure names the entries that were running.

use super::leads_covers::PARSED_FILES;
use super::mechanic_catalog::Registered;
use super::structure_exec::{Parsed, parse_within};
use crate::error_codes::{self, ErrorCode};
use rmcp::ErrorData;
use rusqlite::{Transaction, params_from_iter};
use std::collections::HashSet;
use std::ops::ControlFlow;
use std::time::Instant;
use tree_sitter::{
    Parser, QueryCursor, QueryCursorOptions, QueryCursorState, StreamingIterator, Tree,
};

/// One distinct captured node of one entry.
pub(super) struct Hit<'a> {
    /// Index of the entry in the catalogue's `custom` list.
    pub(super) entry: usize,
    /// Project-relative path.
    pub(super) file: &'a str,
    /// One-based start line of the captured node.
    pub(super) line: i64,
    /// The captured node's source text.
    pub(super) text: &'a str,
}

/// One entry's complete count under a pattern.
#[derive(Debug, Clone, Default)]
pub(super) struct Tally {
    pub(super) count: i64,
    /// Distinct files with at least one hit.
    pub(super) files: i64,
    /// Those files in path order, kept only when asked for (a compact
    /// whole-surface page needs them to union file denominators).
    pub(super) file_list: Vec<String>,
    last_file: Option<String>,
}

/// What a hit callback tells the scan: keep going, or stop every scan now.
pub(super) type Flow = Result<ControlFlow<()>, ErrorData>;

/// The error a spent budget fails the call with, naming the entries that
/// were running (`ids` are `[a-z0-9_]` by the catalogue's id rule).
/// Operational, like a spent sync budget, and permanent for the same scan:
/// the code's remedy names `mechanic remove`, a narrower `pattern` and the
/// knob rather than a retry.
fn budget_error(ids: &[&str]) -> ErrorData {
    ErrorData::internal_error(
        format!(
            "registered mechanic `{}` ran out of the execution time budget \
             (max_structure_execution_time) before every file was read, so no count is returned",
            ids.join("`, `")
        ),
        error_codes::payload(ErrorCode::MechanicBudgetExhausted),
    )
}

/// Counts the entries of `custom` listed in `selected` under `glob`, in one
/// pass per language. The result is indexed like `custom`; an entry not in
/// `selected` keeps an empty tally and must not be reported as counted.
///
/// # Errors
///
/// Fails on a database read error, a grammar that no longer loads, or a
/// spent budget.
pub(super) fn tally(
    tx: &Transaction<'_>,
    custom: &[Registered],
    selected: &[usize],
    glob: Option<&str>,
    deadline: Option<Instant>,
    keep_files: bool,
) -> Result<Vec<Tally>, ErrorData> {
    let mut tallies = vec![Tally::default(); custom.len()];
    scan(tx, custom, selected, glob, deadline, &mut |hit| {
        let tally = tallies.get_mut(hit.entry).ok_or_else(|| {
            super::context::internal_error("counting mechanics", "a hit named no entry")
        })?;
        tally.count = tally.count.saturating_add(1);
        if tally.last_file.as_deref() != Some(hit.file) {
            tally.files = tally.files.saturating_add(1);
            tally.last_file = Some(hit.file.to_owned());
            if keep_files {
                tally.file_list.push(hit.file.to_owned());
            }
        }
        Ok(ControlFlow::Continue(()))
    })?;
    Ok(tallies)
}

/// Runs the entries of `custom` listed in `selected` over their languages'
/// files under `glob`, calling `on_hit` per distinct captured
/// node: for one entry, in path then position order. `on_hit` returning
/// `Break` ends the whole scan early and successfully.
///
/// # Errors
///
/// See [`tally`]; an error from `on_hit` is returned as is.
pub(super) fn scan(
    tx: &Transaction<'_>,
    custom: &[Registered],
    selected: &[usize],
    glob: Option<&str>,
    deadline: Option<Instant>,
    on_hit: &mut dyn FnMut(Hit<'_>) -> Flow,
) -> Result<(), ErrorData> {
    let mut languages: Vec<&str> = Vec::new();
    for index in selected {
        if let Some(registered) = custom.get(*index)
            && !languages.contains(&registered.entry.language.as_str())
        {
            languages.push(&registered.entry.language);
        }
    }
    for language in languages {
        let entries: Vec<usize> = selected
            .iter()
            .copied()
            .filter(|index| {
                custom
                    .get(*index)
                    .is_some_and(|registered| registered.entry.language == language)
            })
            .collect();
        let scan = LanguageScan {
            custom,
            entries: &entries,
            deadline,
        };
        if scan.run(tx, language, glob, on_hit)?.is_break() {
            break;
        }
    }
    Ok(())
}

/// The entries of one language and the call's deadline.
struct LanguageScan<'a> {
    custom: &'a [Registered],
    entries: &'a [usize],
    deadline: Option<Instant>,
}

impl LanguageScan<'_> {
    fn run(
        &self,
        tx: &Transaction<'_>,
        language: &str,
        glob: Option<&str>,
        on_hit: &mut dyn FnMut(Hit<'_>) -> Flow,
    ) -> Result<ControlFlow<()>, ErrorData> {
        let grammar = tree_sitter_language_pack::get_language(language).map_err(|error| {
            super::context::internal_error("loading a registered mechanic's grammar", error)
        })?;
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|error| super::context::internal_error("setting a mechanic grammar", error))?;
        let filter = if glob.is_some() {
            " AND path GLOB ?2"
        } else {
            ""
        };
        let sql = format!(
            "SELECT path, content FROM files WHERE language = ?1 AND {PARSED_FILES} \
             AND content IS NOT NULL{filter} ORDER BY path"
        );
        let mut statement = tx.prepare(&sql).map_err(|error| {
            super::context::internal_error("preparing the mechanic scan", error)
        })?;
        let binds = std::iter::once(language).chain(glob);
        let mut rows = statement
            .query(params_from_iter(binds))
            .map_err(|error| super::context::internal_error("starting the mechanic scan", error))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| super::context::internal_error("reading the mechanic scan", error))?
        {
            let path: String = row
                .get(0)
                .map_err(|error| super::context::internal_error("reading a scanned path", error))?;
            let content: String = row.get(1).map_err(|error| {
                super::context::internal_error("reading scanned content", error)
            })?;
            let tree = self.parse(&mut parser, &content)?;
            for entry in self.entries {
                if self
                    .scan_tree(*entry, &path, &content, &tree, on_hit)?
                    .is_break()
                {
                    return Ok(ControlFlow::Break(()));
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Parses `content` under the deadline with `structure`'s budgeted
    /// parse; a cancelled parse fails the call naming this language's
    /// entries, since the parse is shared by all of them.
    fn parse(&self, parser: &mut Parser, content: &str) -> Result<Tree, ErrorData> {
        if self.past_deadline() {
            return Err(budget_error(&self.ids()));
        }
        match parse_within(parser, content, self.deadline) {
            Parsed::Tree(tree) => Ok(tree),
            Parsed::Cancelled => Err(budget_error(&self.ids())),
            Parsed::NoTree => Err(super::context::internal_error(
                "parsing a file for a registered mechanic",
                "the parser returned no syntax tree",
            )),
        }
    }

    /// The ids of this language's entries, in catalogue order.
    fn ids(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter_map(|index| self.custom.get(*index))
            .map(|registered| registered.entry.id.as_str())
            .collect()
    }

    fn past_deadline(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Runs one entry's query over one tree, reporting each distinct
    /// captured node once.
    fn scan_tree(
        &self,
        entry: usize,
        path: &str,
        content: &str,
        tree: &Tree,
        on_hit: &mut dyn FnMut(Hit<'_>) -> Flow,
    ) -> Result<ControlFlow<()>, ErrorData> {
        let registered = self.custom.get(entry).ok_or_else(|| {
            super::context::internal_error("running mechanics", "an entry index is out of range")
        })?;
        let deadline = self.deadline;
        let mut timed_out = false;
        let mut check = |_state: &QueryCursorState| -> ControlFlow<()> {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                timed_out = true;
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let options = QueryCursorOptions::new().progress_callback(&mut check);
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures_with_options(
            &registered.query,
            tree.root_node(),
            content.as_bytes(),
            options,
        );
        let mut seen = HashSet::new();
        let mut flow = ControlFlow::Continue(());
        while let Some((found, capture_index)) = captures.next() {
            let capture = found.captures.get(*capture_index).ok_or_else(|| {
                super::context::internal_error("reading a hit", "a capture index is out of range")
            })?;
            let node = capture.node;
            if !seen.insert(node.id()) {
                continue;
            }
            let line = i64::try_from(node.start_position().row)
                .map_err(|error| super::context::internal_error("numbering a hit", error))?
                .saturating_add(1);
            let text = content.get(node.byte_range()).ok_or_else(|| {
                super::context::internal_error(
                    "reading a hit",
                    "the capture is not on a char boundary",
                )
            })?;
            flow = on_hit(Hit {
                entry,
                file: path,
                line,
                text,
            })?;
            if flow.is_break() {
                break;
            }
        }
        drop(captures);
        if timed_out {
            return Err(budget_error(&[registered.entry.id.as_str()]));
        }
        Ok(flow)
    }
}
