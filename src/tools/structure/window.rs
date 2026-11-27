//! Memory-bounded, cursor-resumable multi-file scan windowing for `structure`.

use super::source::{INDEXED_WITH_CONTENT, SELECT_CONTENT_SQL};
use crate::tools::context::internal_error;
use crate::tools::sql_predicates::pattern_to_glob;
use rmcp::ErrorData;

pub(super) const MAX_MULTI_FILE_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// One memory-bounded window of a multi-file scan.
#[derive(Debug)]
pub(super) struct SourceWindow {
    pub(super) files: Vec<(String, String)>,
    pub(super) files_not_scanned: usize,
    pub(super) bytes_not_scanned: u64,
    pub(super) files_over_parse_limit: usize,
    pub(super) next_start_after_file: Option<String>,
}

impl SourceWindow {
    pub(super) fn complete(files: Vec<(String, String)>) -> Self {
        Self {
            files,
            files_not_scanned: 0,
            bytes_not_scanned: 0,
            files_over_parse_limit: 0,
            next_start_after_file: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WindowBounds {
    pub(super) window_bytes: usize,
    pub(super) max_parse_bytes: u64,
}

fn file_filter(
    language: &str,
    pattern: Option<&str>,
    after: Option<&str>,
) -> (String, Vec<String>) {
    let mut sql = format!("language = ?1 AND {INDEXED_WITH_CONTENT}");
    let mut params = vec![language.to_owned()];
    if let Some(pattern) = pattern {
        params.push(pattern_to_glob(pattern));
        sql.push_str(" AND path GLOB ?");
        sql.push_str(&params.len().to_string());
    }
    if let Some(after) = after {
        params.push(after.to_owned());
        sql.push_str(" AND path > ?");
        sql.push_str(&params.len().to_string());
    }
    (sql, params)
}

/// Loads matching files in path order strictly after `start_after` while their
/// content fits `bounds.window_bytes`.
pub(super) fn fetch_sources_for_language(
    connection: &rusqlite::Connection,
    language: &str,
    pattern: Option<&str>,
    start_after: Option<&str>,
    bounds: WindowBounds,
) -> Result<SourceWindow, ErrorData> {
    let per_file_limit = bounds.max_parse_bytes.min(bounds.window_bytes as u64);
    let (filter, params) = file_filter(language, pattern, start_after);
    let mut stmt = connection
        .prepare(&format!(
            "SELECT path, octet_length(content) FROM files WHERE {filter} ORDER BY path ASC"
        ))
        .map_err(|error| internal_error("preparing the indexed-file scan", error))?;
    let mut content_stmt = connection
        .prepare(SELECT_CONTENT_SQL)
        .map_err(|error| internal_error("preparing the indexed-file read", error))?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(|error| internal_error("starting the indexed-file scan", error))?;
    let mut files = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut files_over_parse_limit = 0usize;
    let mut last_examined: Option<String> = None;
    let mut window_full = false;
    while let Some(row) = rows
        .next()
        .map_err(|error| internal_error("reading indexed files", error))?
    {
        let path: String = row
            .get(0)
            .map_err(|error| internal_error("reading an indexed file path", error))?;
        let size: u64 = row
            .get(1)
            .map_err(|error| internal_error("reading an indexed file's size", error))?;
        if size > per_file_limit {
            files_over_parse_limit = files_over_parse_limit.checked_add(1).ok_or_else(|| {
                internal_error("counting oversized indexed files", "the count overflowed")
            })?;
            last_examined = Some(path);
            continue;
        }
        if total_bytes
            .checked_add(size)
            .is_none_or(|total| total > bounds.window_bytes as u64)
        {
            window_full = true;
            break;
        }
        let content: String = content_stmt
            .query_row([&path], |row| row.get(0))
            .map_err(|error| internal_error("reading an indexed file's content", error))?;
        total_bytes = total_bytes
            .checked_add(content.len() as u64)
            .ok_or_else(|| {
                internal_error("counting the structure window", "the byte count overflowed")
            })?;
        last_examined = Some(path.clone());
        files.push((path, content));
    }
    let (files_not_scanned, bytes_not_scanned) = if window_full {
        count_unscanned(connection, language, pattern, last_examined.as_deref())?
    } else {
        (0, 0)
    };
    Ok(SourceWindow {
        next_start_after_file: if files_not_scanned > 0 {
            last_examined
        } else {
            None
        },
        files,
        files_not_scanned,
        bytes_not_scanned,
        files_over_parse_limit,
    })
}

fn count_unscanned(
    connection: &rusqlite::Connection,
    language: &str,
    pattern: Option<&str>,
    after: Option<&str>,
) -> Result<(usize, u64), ErrorData> {
    let (rest, rest_params) = file_filter(language, pattern, after);
    connection
        .query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(octet_length(content)), 0) FROM files WHERE {rest}"
            ),
            rusqlite::params_from_iter(rest_params.iter()),
            |row| Ok((row.get::<_, usize>(0)?, row.get::<_, u64>(1)?)),
        )
        .map_err(|error| internal_error("counting files past the scan window", error))
}
