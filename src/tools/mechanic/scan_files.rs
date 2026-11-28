//! SQLite indexed-file counting and streaming for registered-mechanic scans.

use crate::tools::context::internal_error;
use crate::tools::sql_predicates::PARSED_FILES;
use rmcp::ErrorData;
use rusqlite::{Transaction, params_from_iter};

fn file_filter(glob: Option<&str>) -> String {
    let filter = if glob.is_some() {
        " AND slugaudit_path_match(path, ?2)"
    } else {
        ""
    };
    format!("language = ?1 AND {PARSED_FILES} AND content IS NOT NULL{filter}")
}

/// Counts indexed, parsed files across `languages` matching `glob`.
pub(super) fn count_files(
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
            .map_err(|error| internal_error("counting the files to scan", error))?;
        let count = u64::try_from(count)
            .map_err(|error| internal_error("counting the files to scan", error))?;
        total = total
            .checked_add(count)
            .ok_or_else(|| internal_error("counting the files to scan", "the count overflowed"))?;
    }
    Ok(total)
}

/// Streams `(path, content)` rows of `language` matching `glob` in path order
/// until exhausted (`Ok(None)`) or `on_file` returns `Ok(Some(T))`.
pub(super) fn for_each_file<T>(
    tx: &Transaction<'_>,
    language: &str,
    glob: Option<&str>,
    mut on_file: impl FnMut(String, String) -> Result<Option<T>, ErrorData>,
) -> Result<Option<T>, ErrorData> {
    let sql = format!(
        "SELECT path, content FROM files WHERE {} ORDER BY path",
        file_filter(glob)
    );
    let mut statement = tx
        .prepare(&sql)
        .map_err(|error| internal_error("preparing the mechanic scan", error))?;
    let mut rows = statement
        .query(params_from_iter(std::iter::once(language).chain(glob)))
        .map_err(|error| internal_error("starting the mechanic scan", error))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| internal_error("reading the mechanic scan", error))?
    {
        let path: String = row
            .get(0)
            .map_err(|error| internal_error("reading a scanned path", error))?;
        let content: String = row
            .get(1)
            .map_err(|error| internal_error("reading scanned content", error))?;
        if let Some(stopped) = on_file(path, content)? {
            return Ok(Some(stopped));
        }
    }
    Ok(None)
}
