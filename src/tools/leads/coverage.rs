//! Language-coverage denominators and unparsed-file counts for `leads`.

use super::mechanics::Reads;
use crate::tools::PARSER_FAILURE_PREDICATE;
use crate::tools::context::internal_error;
use crate::tools::sql_predicates::PARSED_FILES;
use rmcp::ErrorData;
use rusqlite::{Transaction, params_from_iter};

const ALL_FILES: &str = "language IS NOT NULL";

pub(in crate::tools) struct LanguagesRead {
    parsed: Vec<String>,
    paths: Vec<String>,
}

impl LanguagesRead {
    pub(super) fn covers(&self, reads: Reads) -> Vec<String> {
        match reads {
            Reads::Calls => self
                .parsed
                .iter()
                .filter(|lang| crate::evidence::call_languages().any(|call| call == lang.as_str()))
                .cloned()
                .collect(),
            Reads::ParsedFiles => self.parsed.clone(),
            Reads::Paths => self.paths.clone(),
        }
    }

    pub(super) fn covers_language(&self, language: &str) -> Vec<String> {
        self.parsed
            .iter()
            .filter(|parsed| parsed.as_str() == language)
            .cloned()
            .collect()
    }
}

fn path_glob_clause(glob: Option<&str>) -> &'static str {
    if glob.is_some() {
        " AND path GLOB ?1"
    } else {
        ""
    }
}

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
    let sql = format!(
        "SELECT DISTINCT language FROM files WHERE {predicate}{} ORDER BY language",
        path_glob_clause(glob)
    );
    let mut stmt = tx
        .prepare(&sql)
        .map_err(|error| internal_error("preparing covered languages", error))?;
    stmt.query_map(params_from_iter(glob.iter()), |row| row.get(0))
        .map_err(|error| internal_error("reading covered languages", error))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|error| internal_error("reading covered languages", error))
}

pub(super) fn files_not_parsed(tx: &Transaction<'_>, glob: Option<&str>) -> Result<i64, ErrorData> {
    let sql = format!(
        "SELECT count(*) FROM files WHERE {PARSER_FAILURE_PREDICATE} AND language IS NOT NULL{}",
        path_glob_clause(glob)
    );
    tx.query_row(&sql, params_from_iter(glob.iter()), |row| row.get(0))
        .map_err(|error| internal_error("counting files not parsed", error))
}
