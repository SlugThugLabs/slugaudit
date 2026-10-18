use super::context::{SyncedProject, with_verified_read};
use super::structure::StructureRequest;
use crate::error_codes::{self, ErrorCode};
use rmcp::ErrorData;

pub(super) const MAX_TEXT_BYTES: usize = 1_000_000;
pub(super) const MAX_SNIPPET_BYTES: usize = 120;
pub(super) const MAX_MULTI_FILE_TOTAL_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn fetch_source(
    connection: &rusqlite::Connection,
    file: &str,
) -> Result<(String, String), ErrorData> {
    connection
        .query_row(
            "SELECT content, language FROM files WHERE path = ?1",
            [file],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| ErrorData::invalid_params(format!("{file}: {error}"), None))
        .and_then(|(content, language): (Option<String>, Option<String>)| {
            let c = content.ok_or_else(|| {
                ErrorData::invalid_params(
                    format!("{file} has no indexed source content"),
                    error_codes::payload(ErrorCode::FileNotIndexed),
                )
            })?;
            let l = language.ok_or_else(|| {
                ErrorData::invalid_params(format!("{file} has no detected language"), None)
            })?;
            Ok((c, l))
        })
}

/// One memory-bounded window of a multi-file scan: the files loaded, and
/// what the window cut left unloaded (never silently).
pub(super) struct SourceWindow {
    /// `(path, content)` in path order.
    pub(super) files: Vec<(String, String)>,
    /// Matching files past the window that were not loaded.
    pub(super) files_not_scanned: usize,
    /// Content bytes of those files.
    pub(super) bytes_not_scanned: u64,
    /// The last path loaded, to pass as `start_after_file` for the next
    /// window; `None` when the window reached the end of the matching files.
    pub(super) next_start_after_file: Option<String>,
}

impl SourceWindow {
    /// A window that holds everything there is (a single named file).
    pub(super) fn complete(files: Vec<(String, String)>) -> Self {
        Self {
            files,
            files_not_scanned: 0,
            bytes_not_scanned: 0,
            next_start_after_file: None,
        }
    }
}

/// The `WHERE` clause and positional parameters selecting indexed files of
/// `language`, optionally filtered by `pattern` and strictly after a path.
fn file_filter(
    language: &str,
    pattern: Option<&str>,
    after: Option<&str>,
) -> (String, Vec<String>) {
    let mut sql = "language = ?1 AND file_kind = 'indexed' AND content IS NOT NULL".to_owned();
    let mut params = vec![language.to_owned()];
    if let Some(pattern) = pattern {
        params.push(if pattern.contains(['*', '?']) {
            pattern.to_owned()
        } else {
            format!("*{pattern}*")
        });
        sql.push_str(&format!(" AND path GLOB ?{}", params.len()));
    }
    if let Some(after) = after {
        params.push(after.to_owned());
        sql.push_str(&format!(" AND path > ?{}", params.len()));
    }
    (sql, params)
}

fn internal(error: rusqlite::Error) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// Loads matching files in path order, strictly after `start_after`, until
/// their content reaches `window_bytes` (the file that crosses it is kept, so
/// every window makes progress). What lies past the window is counted.
pub(super) fn fetch_sources_for_language(
    connection: &rusqlite::Connection,
    language: &str,
    pattern: Option<&str>,
    start_after: Option<&str>,
    window_bytes: usize,
) -> Result<SourceWindow, ErrorData> {
    let (filter, params) = file_filter(language, pattern, start_after);
    let mut stmt = connection
        .prepare(&format!(
            "SELECT path, content FROM files WHERE {filter} ORDER BY path ASC"
        ))
        .map_err(internal)?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(internal)?;
    let mut files = Vec::new();
    let mut total_bytes: usize = 0;
    let mut window_full = false;
    while let Some(row) = rows.next().map_err(internal)? {
        let path: String = row.get(0).map_err(internal)?;
        let content: String = row.get(1).map_err(internal)?;
        total_bytes = total_bytes.saturating_add(content.len());
        files.push((path, content));
        if total_bytes >= window_bytes {
            window_full = true;
            break;
        }
    }
    let last = files.last().map(|(path, _)| path.clone());
    let (mut files_not_scanned, mut bytes_not_scanned) = (0usize, 0u64);
    if window_full {
        let (rest, rest_params) = file_filter(language, pattern, last.as_deref());
        (files_not_scanned, bytes_not_scanned) = connection
            .query_row(
                &format!(
                    "SELECT COUNT(*), COALESCE(SUM(length(CAST(content AS BLOB))), 0) \
                     FROM files WHERE {rest}"
                ),
                rusqlite::params_from_iter(rest_params.iter()),
                |row| Ok((row.get::<_, usize>(0)?, row.get::<_, u64>(1)?)),
            )
            .map_err(internal)?;
    }
    Ok(SourceWindow {
        files,
        files_not_scanned,
        bytes_not_scanned,
        next_start_after_file: if files_not_scanned > 0 { last } else { None },
    })
}

/// The grammar to use and the window of `(path, content)` pairs to scan: the
/// one named file (language detected from the index), or the next memory
/// window of indexed files of the requested language matching `pattern`.
pub(super) fn load_targets(
    synced: &SyncedProject,
    request: &StructureRequest,
    window_bytes: usize,
) -> Result<(String, SourceWindow), ErrorData> {
    if let Some(file) = &request.file {
        let (content, detected) = with_verified_read(synced, |tx| fetch_source(tx, file))?;
        let files = vec![(file.clone(), content)];
        return Ok((detected, SourceWindow::complete(files)));
    }
    let language = request.language.as_deref().ok_or_else(|| {
        ErrorData::invalid_params(
            "structure request without 'file' must specify 'language'",
            None,
        )
    })?;
    let window = with_verified_read(synced, |tx| {
        fetch_sources_for_language(
            tx,
            language,
            request.pattern.as_deref(),
            request.start_after_file.as_deref(),
            window_bytes,
        )
    })?;
    Ok((language.to_owned(), window))
}

pub(super) fn truncate_text(text: &str) -> (String, bool) {
    if text.len() <= MAX_TEXT_BYTES {
        return (text.to_owned(), false);
    }
    let mut end = MAX_TEXT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

pub(super) fn truncate_snippet(text: &str, max_bytes: usize) -> (String, bool) {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.len() <= max_bytes {
        (first_line.to_owned(), text.lines().nth(1).is_some())
    } else {
        let mut end = max_bytes;
        while end > 0 && !first_line.is_char_boundary(end) {
            end -= 1;
        }
        (first_line[..end].to_owned(), true)
    }
}
