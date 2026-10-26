//! What each `leads` mechanic was measured over: the `covers` list on every
//! count, built from the languages of the files the mechanic actually read
//! under the request's `pattern`.
//!
//! Split from `leads_query.rs` (the statements that count and page rows) so
//! "which languages did this zero check" is answered in one place, from the
//! index itself and from the call-language list the evidence layer derives
//! from its embedded queries — never from a hand-kept list that could drift.

use super::leads_mechanics::Reads;
use rmcp::ErrorData;
use rusqlite::{Transaction, params_from_iter};

/// Files whose grammar ran to a tree, so every evidence walker read them.
/// Excludes what `files_not_parsed` counts, prose, binary and skipped files.
pub(super) const PARSED_FILES: &str = "file_kind = 'indexed' AND language IS NOT NULL \
    AND parse_outcome IN ('Succeeded', 'SyntaxErrors')";

/// Every file with a detected language: a path mechanic reads them all.
const ALL_FILES: &str = "language IS NOT NULL";

/// The sorted, distinct languages of one revision's files, under a pattern,
/// per kind of read.
pub(super) struct LanguagesRead {
    parsed: Vec<String>,
    paths: Vec<String>,
}

impl LanguagesRead {
    /// The `covers` list for a mechanic that reads `reads`.
    pub(super) fn covers(&self, reads: Reads) -> Vec<String> {
        match reads {
            Reads::Calls => self
                .parsed
                .iter()
                .filter(|language| {
                    crate::evidence::call_languages().any(|call| call == language.as_str())
                })
                .cloned()
                .collect(),
            Reads::ParsedFiles => self.parsed.clone(),
            Reads::Paths => self.paths.clone(),
        }
    }

    /// The `covers` list for a registered mechanic: its one language, when
    /// a parsed file of it is under the pattern.
    pub(super) fn covers_language(&self, language: &str) -> Vec<String> {
        self.parsed
            .iter()
            .filter(|parsed| parsed.as_str() == language)
            .cloned()
            .collect()
    }
}

/// Reads both language sets under `glob` (already compiled by
/// `leads_query::compile_pattern`; `None` means every file).
pub(super) fn languages_read(
    tx: &Transaction<'_>,
    glob: Option<&str>,
) -> Result<LanguagesRead, ErrorData> {
    Ok(LanguagesRead {
        parsed: distinct_languages(tx, PARSED_FILES, glob)?,
        paths: distinct_languages(tx, ALL_FILES, glob)?,
    })
}

fn distinct_languages(
    tx: &Transaction<'_>,
    predicate: &str,
    glob: Option<&str>,
) -> Result<Vec<String>, ErrorData> {
    let filter = if glob.is_some() {
        " AND path GLOB ?1"
    } else {
        ""
    };
    let sql =
        format!("SELECT DISTINCT language FROM files WHERE {predicate}{filter} ORDER BY language");
    let mut statement = tx
        .prepare(&sql)
        .map_err(|error| super::context::internal_error("preparing covered languages", error))?;
    let languages = statement
        .query_map(params_from_iter(glob.iter()), |row| row.get(0))
        .map_err(|error| super::context::internal_error("reading covered languages", error))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|error| super::context::internal_error("reading covered languages", error))?;
    Ok(languages)
}
