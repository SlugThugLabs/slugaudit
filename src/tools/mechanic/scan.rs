// slugaudit-line-exception: approved-by=split-judge; reason=Single cohesive budgeted Tree-sitter scan engine for registered mechanics with modular units and no internal duplication.
//! Budgeted Tree-sitter scan engine for registered session mechanics.

use super::catalog::Registered;
use super::scan_files::{count_files, for_each_file};
use crate::parse::budget::{Parsed, parse_within};
use crate::tools::context::internal_error;
use crate::tools::grammar::{parser_for, run_budgeted_captures};
use crate::tools::leads::MAX_DETAIL_CHARS;
use crate::tools::page::PageLimit;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::collections::HashSet;
use std::time::Instant;
use tree_sitter::{Parser, QueryCursor, QueryMatch, Tree};

/// One distinct captured node of one entry.
pub(in crate::tools) struct Hit<'a> {
    pub(in crate::tools) entry: usize,
    pub(in crate::tools) file: &'a str,
    pub(in crate::tools) line: i64,
    pub(in crate::tools) text: &'a str,
}

/// Where a stopped scan got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::tools) struct Stop {
    pub(in crate::tools) limited_by: PageLimit,
    pub(in crate::tools) files_scanned: u64,
    pub(in crate::tools) files_total: u64,
    pub(in crate::tools) covered_languages: HashSet<String>,
}

/// How a scan ended.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::tools) enum ScanEnd {
    Finished,
    Stopped(Stop),
}

/// Runs `selected` entries of `custom` over their languages' indexed files.
pub(in crate::tools) fn scan(
    tx: &Transaction<'_>,
    custom: &[Registered],
    selected: &[usize],
    glob: Option<&str>,
    deadline: Option<Instant>,
    on_hit: &mut dyn FnMut(Hit<'_>) -> Result<(), ErrorData>,
) -> Result<ScanEnd, ErrorData> {
    let groups = group_by_language(custom, selected);
    let languages: Vec<&str> = groups.iter().map(|(lang, _)| *lang).collect();
    let mut scanned = 0_u64;
    let mut covered_languages = HashSet::new();
    for (language, entries) in &groups {
        let runner = LanguageScan { entries, deadline };
        let before = scanned;
        let stopped = runner.run(tx, language, glob, &mut scanned, on_hit)?;
        if scanned > before {
            covered_languages.insert((*language).to_owned());
        }
        if let Some(limited_by) = stopped {
            return Ok(ScanEnd::Stopped(Stop {
                limited_by,
                files_scanned: scanned,
                files_total: count_files(tx, &languages, glob)?,
                covered_languages,
            }));
        }
    }
    Ok(ScanEnd::Finished)
}

fn group_by_language<'a>(
    custom: &'a [Registered],
    selected: &[usize],
) -> Vec<(&'a str, Vec<(usize, &'a Registered)>)> {
    let mut groups: Vec<(&'a str, Vec<(usize, &'a Registered)>)> = Vec::new();
    for &index in selected {
        let Some(registered) = custom.get(index) else {
            continue;
        };
        let lang = registered.entry.language.as_str();
        if let Some((_, entries)) = groups.iter_mut().find(|(seen, _)| *seen == lang) {
            entries.push((index, registered));
        } else {
            groups.push((lang, vec![(index, registered)]));
        }
    }
    groups
}

enum FileScan {
    Complete(Vec<Held>),
    Halted(PageLimit),
}

struct Held {
    entry: usize,
    line: i64,
    text: String,
}

struct LanguageScan<'a> {
    entries: &'a [(usize, &'a Registered)],
    deadline: Option<Instant>,
}

impl LanguageScan<'_> {
    fn run(
        &self,
        tx: &Transaction<'_>,
        language: &str,
        glob: Option<&str>,
        scanned: &mut u64,
        on_hit: &mut dyn FnMut(Hit<'_>) -> Result<(), ErrorData>,
    ) -> Result<Option<PageLimit>, ErrorData> {
        let mut parser = parser_for(language, "setting a mechanic grammar")?;
        for_each_file(tx, language, glob, |path, content| {
            let held = match self.scan_file(&mut parser, &content)? {
                FileScan::Complete(held) => held,
                FileScan::Halted(limited_by) => return Ok(Some(limited_by)),
            };
            for hit in &held {
                on_hit(Hit {
                    entry: hit.entry,
                    file: &path,
                    line: hit.line,
                    text: &hit.text,
                })?;
            }
            *scanned = scanned
                .checked_add(1)
                .ok_or_else(|| internal_error("counting scanned files", "the count overflowed"))?;
            Ok(None)
        })
    }

    fn scan_file(&self, parser: &mut Parser, content: &str) -> Result<FileScan, ErrorData> {
        let Some(tree) = self.parse(parser, content)? else {
            return Ok(FileScan::Halted(PageLimit::TimeBudget));
        };
        let mut held = Vec::new();
        for &(entry, registered) in self.entries {
            if let Some(limit) = self.scan_tree(entry, registered, content, &tree, &mut held)? {
                return Ok(FileScan::Halted(limit));
            }
        }
        Ok(FileScan::Complete(held))
    }

    fn parse(&self, parser: &mut Parser, content: &str) -> Result<Option<Tree>, ErrorData> {
        if self.past_deadline() {
            return Ok(None);
        }
        match parse_within(parser, content, || self.past_deadline()) {
            Parsed::Tree(tree) => Ok(Some(tree)),
            Parsed::Cancelled => Ok(None),
            Parsed::NoTree => Err(internal_error(
                "parsing a file for a registered mechanic",
                "the parser returned no syntax tree",
            )),
        }
    }

    fn past_deadline(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn scan_tree(
        &self,
        entry: usize,
        registered: &Registered,
        content: &str,
        tree: &Tree,
        held: &mut Vec<Held>,
    ) -> Result<Option<PageLimit>, ErrorData> {
        let mut cursor = QueryCursor::new();
        let mut seen = HashSet::new();
        let outcome = run_budgeted_captures(
            &mut cursor,
            &registered.query,
            tree.root_node(),
            content.as_bytes(),
            self.deadline,
            |found, capture_index| {
                hold_capture(entry, content, found, capture_index, &mut seen, held)
            },
        )?;
        if outcome.timed_out {
            return Ok(Some(PageLimit::TimeBudget));
        }
        if outcome.match_limit_exceeded {
            return Ok(Some(PageLimit::QueryMatchLimit));
        }
        Ok(None)
    }
}

fn hold_capture(
    entry: usize,
    content: &str,
    found: &QueryMatch<'_, '_>,
    capture_index: usize,
    seen: &mut HashSet<usize>,
    held: &mut Vec<Held>,
) -> Result<(), ErrorData> {
    let capture = found
        .captures()
        .get(capture_index)
        .ok_or_else(|| internal_error("reading a hit", "a capture index is out of range"))?;
    let node = capture.node;
    if !seen.insert(node.id()) {
        return Ok(());
    }
    let line = i64::try_from(node.start_position().row)
        .map_err(|error| internal_error("numbering a hit", error))?
        .checked_add(1)
        .ok_or_else(|| internal_error("numbering a hit", "the line number overflowed"))?;
    let text = content
        .get(node.byte_range())
        .ok_or_else(|| internal_error("reading a hit", "the capture is not on a char boundary"))?;
    held.push(Held {
        entry,
        line,
        text: text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
    });
    Ok(())
}
