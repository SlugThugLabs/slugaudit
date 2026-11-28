//! Single-file source loading, target selection, and indexed-language checks
//! for `structure`.

use super::StructureRequest;
pub(super) use super::window::{
    MAX_MULTI_FILE_TOTAL_BYTES, SourceWindow, WindowBounds, fetch_sources_for_language,
};
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::parse::PROSE_LANGUAGE;
use crate::tools::context::internal_error;
use crate::tools::grammar::prose_refusal_reason;
use rmcp::ErrorData;
use rusqlite::OptionalExtension;

pub(super) const INDEXED_WITH_CONTENT: &str = "file_kind = 'indexed' AND content IS NOT NULL";
pub(super) const SELECT_CONTENT_SQL: &str = "SELECT content FROM files WHERE path = ?1";

pub(super) fn prose_not_parsed_error(language: &str) -> ErrorData {
    invalid_params_with_code(prose_refusal_reason(language), ErrorCode::InvalidInput)
}

/// Loads `(content, language)` for a single named `file`.
pub(super) fn fetch_source(
    connection: &rusqlite::Connection,
    file: &str,
    explicit_language: Option<&str>,
    max_parse_bytes: u64,
) -> Result<(String, String), ErrorData> {
    let row: Option<(Option<i64>, Option<String>)> = connection
        .query_row(
            "SELECT octet_length(content), language FROM files WHERE path = ?1",
            [file],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| internal_error("reading the indexed file row", error))?;
    let Some((content_bytes, detected_language)) = row else {
        return Err(invalid_params_with_code(
            format!("{file} is not indexed in the current revision"),
            ErrorCode::FileNotIndexed,
        ));
    };
    if detected_language.as_deref() == Some(PROSE_LANGUAGE) {
        return Err(prose_not_parsed_error(PROSE_LANGUAGE));
    }
    let effective_language = explicit_language.or(detected_language.as_deref());
    if effective_language == Some(PROSE_LANGUAGE) {
        return Err(prose_not_parsed_error(PROSE_LANGUAGE));
    }
    let content_bytes = content_bytes
        .ok_or_else(|| {
            invalid_params_with_code(
                format!("{file} has no indexed source content"),
                ErrorCode::FileNotIndexed,
            )
        })
        .and_then(|bytes| {
            u64::try_from(bytes)
                .map_err(|error| internal_error("reading the indexed file size", error))
        })?;
    let language = effective_language.map(str::to_owned).ok_or_else(|| {
        invalid_params_with_code(
            format!("{file} has no detected language"),
            ErrorCode::InvalidInput,
        )
    })?;
    if content_bytes > max_parse_bytes {
        return Err(invalid_params_with_code(
            format!(
                "{file} is {content_bytes} bytes, over max_parse_bytes ({max_parse_bytes}): it is \
                 stored and searchable via `query` but not parsed, so `structure` cannot run on it"
            ),
            ErrorCode::InvalidInput,
        ));
    }
    let content: String = connection
        .query_row(SELECT_CONTENT_SQL, [file], |row| row.get(0))
        .map_err(|error| internal_error("reading the indexed file content", error))?;
    Ok((content, language))
}

pub(super) fn load_targets(
    connection: &rusqlite::Connection,
    request: &StructureRequest,
    bounds: WindowBounds,
) -> Result<(String, SourceWindow), ErrorData> {
    if let Some(file) = &request.file {
        let (content, detected) = fetch_source(
            connection,
            file,
            request.language.as_deref(),
            bounds.max_parse_bytes,
        )?;
        return Ok((
            detected,
            SourceWindow::complete(vec![(file.clone(), content)]),
        ));
    }
    let language = request.language.as_deref().ok_or_else(|| {
        invalid_params_with_code(
            "structure request without 'file' must specify 'language'",
            ErrorCode::InvalidInput,
        )
    })?;
    ensure_indexed_language(connection, language)?;
    let window = fetch_sources_for_language(
        connection,
        language,
        request.pattern.as_deref(),
        request.start_after_file.as_deref(),
        bounds,
    )?;
    Ok((language.to_owned(), window))
}

fn ensure_indexed_language(
    connection: &rusqlite::Connection,
    language: &str,
) -> Result<(), ErrorData> {
    let scannable = format!("{INDEXED_WITH_CONTENT} AND language IS NOT NULL AND language <> ?1");
    let present: bool = connection
        .query_row(
            &format!("SELECT EXISTS (SELECT 1 FROM files WHERE language = ?2 AND {scannable})"),
            [PROSE_LANGUAGE, language],
            |row| row.get(0),
        )
        .map_err(|error| internal_error("checking the requested language is indexed", error))?;
    if present {
        return Ok(());
    }
    let listed = list_indexed_languages(connection, &scannable)?;
    Err(invalid_params_with_code(
        format!(
            "unknown language: no indexed file in this project has the requested language; \
             languages indexed here: {listed}"
        ),
        ErrorCode::UnknownLanguage,
    ))
}

fn list_indexed_languages(
    connection: &rusqlite::Connection,
    scannable: &str,
) -> Result<String, ErrorData> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT DISTINCT language FROM files WHERE {scannable} ORDER BY language"
        ))
        .map_err(|error| internal_error("preparing the indexed-language list", error))?;
    let indexed = statement
        .query_map([PROSE_LANGUAGE], |row| row.get::<_, String>(0))
        .and_then(Iterator::collect::<rusqlite::Result<Vec<String>>>)
        .map_err(|error| internal_error("listing the indexed languages", error))?;
    Ok(if indexed.is_empty() {
        "none (no parsed source files are indexed)".to_owned()
    } else {
        indexed.join(", ")
    })
}
