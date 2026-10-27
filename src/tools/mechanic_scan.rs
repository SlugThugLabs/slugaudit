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
//! A spent budget stops the scan: the result so far is returned as
//! [`ScanEnd::Stopped`], naming the last file fully scanned, so a partial
//! count is never mistaken for a complete one. A file the budget cut in the
//! middle contributes no hit at all (its hits are held back until the file
//! is done), so resuming after the reported file repeats nothing. Only the
//! entries a call needs are scanned (`leads` naming a built-in scans none),
//! so one slow entry cannot stop calls that never asked for it.

use super::leads_covers::PARSED_FILES;
use super::leads_query::MAX_DETAIL_CHARS;
use super::mechanic_catalog::Registered;
use crate::error_codes::{self, ErrorCode};
use crate::parse::budget::{Parsed, parse_within};
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
    /// The captured node's source text, clipped to `MAX_DETAIL_CHARS` plus
    /// one character (enough for a consumer to tell it was clipped).
    pub(super) text: &'a str,
}

/// One entry's complete count under a pattern.
#[derive(Debug, Clone, Default)]
pub(super) struct Tally {
    pub(super) count: i64,
    /// Distinct files with at least one hit.
    pub(super) files: i64,
    last_file: Option<String>,
}

impl Tally {
    /// Counts one hit in `file`; hits arrive file-contiguous, in path order.
    pub(super) fn record(&mut self, file: &str) {
        self.count = self.count.saturating_add(1);
        if self.last_file.as_deref() != Some(file) {
            self.files = self.files.saturating_add(1);
            self.last_file = Some(file.to_owned());
        }
    }
}

/// Where a scan the budget stopped got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stop {
    /// The last file fully scanned in the language that was running; hits of
    /// the file the budget cut are not reported.
    pub(super) resume_after_file: Option<String>,
    /// Files fully scanned, over every language of the scan.
    pub(super) files_scanned: u64,
    /// Files the scan selects, over every language of the scan.
    pub(super) files_total: u64,
}

/// How a scan ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ScanEnd {
    /// Every selected file was read.
    Finished,
    /// The call deadline stopped the scan.
    Stopped(Stop),
}

/// The error a scan that cannot report a partial result fails the call with,
/// naming the entries that were running (`ids` are `[a-z0-9_]` by the
/// catalogue's id rule). Operational, like a spent sync budget, and permanent
/// for the same scan: the code's remedy names `mechanic remove`, a narrower
/// `pattern` and the knob rather than a retry.
pub(super) fn budget_error(ids: &[&str]) -> ErrorData {
    ErrorData::internal_error(
        format!(
            "registered mechanic `{}` ran out of the execution time budget \
             (max_structure_execution_time) before every file was read, so no count is returned",
            ids.join("`, `")
        ),
        error_codes::payload(ErrorCode::MechanicBudgetExhausted),
    )
}

/// Runs the entries of `custom` listed in `selected` over their languages'
/// files under `glob`, calling `on_hit` per distinct captured node: for one
/// entry, in path then position order. The hits of one file reach `on_hit`
/// only once the file is fully scanned.
///
/// # Errors
///
/// Fails on a database read error or a grammar that no longer loads; an
/// error from `on_hit` is returned as is. A spent budget is not an error: it
/// ends the scan as [`ScanEnd::Stopped`].
pub(super) fn scan(
    tx: &Transaction<'_>,
    custom: &[Registered],
    selected: &[usize],
    glob: Option<&str>,
    deadline: Option<Instant>,
    on_hit: &mut dyn FnMut(Hit<'_>) -> Result<(), ErrorData>,
) -> Result<ScanEnd, ErrorData> {
    let mut languages: Vec<&str> = Vec::new();
    for index in selected {
        if let Some(registered) = custom.get(*index)
            && !languages.contains(&registered.entry.language.as_str())
        {
            languages.push(&registered.entry.language);
        }
    }
    let mut scanned = 0_u64;
    for language in &languages {
        let entries: Vec<usize> = selected
            .iter()
            .copied()
            .filter(|index| {
                custom
                    .get(*index)
                    .is_some_and(|registered| registered.entry.language == *language)
            })
            .collect();
        let scan = LanguageScan {
            custom,
            entries: &entries,
            deadline,
        };
        if let Some(Halted { after }) = scan.run(tx, language, glob, &mut scanned, on_hit)? {
            return Ok(ScanEnd::Stopped(Stop {
                resume_after_file: after,
                files_scanned: scanned,
                files_total: count_files(tx, &languages, glob)?,
            }));
        }
    }
    Ok(ScanEnd::Finished)
}

/// The files a scan of `languages` reads: indexed, grammar ran, content kept.
fn file_filter(glob: Option<&str>) -> String {
    let filter = if glob.is_some() {
        " AND path GLOB ?2"
    } else {
        ""
    };
    format!("language = ?1 AND {PARSED_FILES} AND content IS NOT NULL{filter}")
}

/// How many files the scan of `languages` selects under `glob`.
fn count_files(
    tx: &Transaction<'_>,
    languages: &[&str],
    glob: Option<&str>,
) -> Result<u64, ErrorData> {
    let sql = format!("SELECT count(*) FROM files WHERE {}", file_filter(glob));
    let mut total = 0_u64;
    for language in languages {
        let count: i64 = tx
            .query_row(
                &sql,
                params_from_iter(std::iter::once(*language).chain(glob)),
                |row| row.get(0),
            )
            .map_err(|error| super::context::internal_error("counting the files to scan", error))?;
        let count = u64::try_from(count)
            .map_err(|error| super::context::internal_error("counting the files to scan", error))?;
        total = total.saturating_add(count);
    }
    Ok(total)
}

/// Where the deadline stopped one language's scan.
struct Halted {
    /// The last file fully scanned, `None` when not even one was.
    after: Option<String>,
}

/// One hit held back until its file is done.
struct Held {
    entry: usize,
    line: i64,
    text: String,
}

/// The entries of one language and the call's deadline.
struct LanguageScan<'a> {
    custom: &'a [Registered],
    entries: &'a [usize],
    deadline: Option<Instant>,
}

impl LanguageScan<'_> {
    /// Scans the language's files in path order. `None` means every file was
    /// read; `Some` means the deadline stopped the scan. `scanned` counts the
    /// files fully scanned.
    fn run(
        &self,
        tx: &Transaction<'_>,
        language: &str,
        glob: Option<&str>,
        scanned: &mut u64,
        on_hit: &mut dyn FnMut(Hit<'_>) -> Result<(), ErrorData>,
    ) -> Result<Option<Halted>, ErrorData> {
        let grammar = tree_sitter_language_pack::get_language(language).map_err(|error| {
            super::context::internal_error("loading a registered mechanic's grammar", error)
        })?;
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|error| super::context::internal_error("setting a mechanic grammar", error))?;
        let sql = format!(
            "SELECT path, content FROM files WHERE {} ORDER BY path",
            file_filter(glob)
        );
        let mut statement = tx.prepare(&sql).map_err(|error| {
            super::context::internal_error("preparing the mechanic scan", error)
        })?;
        let binds = std::iter::once(language).chain(glob);
        let mut rows = statement
            .query(params_from_iter(binds))
            .map_err(|error| super::context::internal_error("starting the mechanic scan", error))?;
        let mut last_done: Option<String> = None;
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
            let Some(held) = self.scan_file(&mut parser, &content)? else {
                return Ok(Some(Halted { after: last_done }));
            };
            for hit in &held {
                on_hit(Hit {
                    entry: hit.entry,
                    file: &path,
                    line: hit.line,
                    text: &hit.text,
                })?;
            }
            *scanned = scanned.saturating_add(1);
            last_done = Some(path);
        }
        Ok(None)
    }

    /// Parses one file and runs every entry over it. `None` when the deadline
    /// cut the file, in which case none of its hits are returned.
    fn scan_file(
        &self,
        parser: &mut Parser,
        content: &str,
    ) -> Result<Option<Vec<Held>>, ErrorData> {
        let Some(tree) = self.parse(parser, content)? else {
            return Ok(None);
        };
        let mut held = Vec::new();
        for entry in self.entries {
            if !self.scan_tree(*entry, content, &tree, &mut held)? {
                return Ok(None);
            }
        }
        Ok(Some(held))
    }

    /// Parses `content` under the deadline with `structure`'s budgeted
    /// parse; `None` when the deadline cancelled it.
    fn parse(&self, parser: &mut Parser, content: &str) -> Result<Option<Tree>, ErrorData> {
        if self.past_deadline() {
            return Ok(None);
        }
        match parse_within(parser, content, || self.past_deadline()) {
            Parsed::Tree(tree) => Ok(Some(tree)),
            Parsed::Cancelled => Ok(None),
            Parsed::NoTree => Err(super::context::internal_error(
                "parsing a file for a registered mechanic",
                "the parser returned no syntax tree",
            )),
        }
    }

    fn past_deadline(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Runs one entry's query over one tree, holding each distinct captured
    /// node once. `false` when the deadline cut the query.
    fn scan_tree(
        &self,
        entry: usize,
        content: &str,
        tree: &Tree,
        held: &mut Vec<Held>,
    ) -> Result<bool, ErrorData> {
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
            held.push(Held {
                entry,
                line,
                text: text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
            });
        }
        drop(captures);
        Ok(!timed_out)
    }
}
