use rmcp::ErrorData;

pub(super) const MAX_TEXT_BYTES: usize = 1_000_000;
pub(super) const MAX_SNIPPET_BYTES: usize = 120;

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
                ErrorData::invalid_params(format!("{file} has no indexed source content"), None)
            })?;
            let l = language.ok_or_else(|| {
                ErrorData::invalid_params(format!("{file} has no detected language"), None)
            })?;
            Ok((c, l))
        })
}

pub(super) fn fetch_sources_for_language(
    connection: &rusqlite::Connection,
    language: &str,
    pattern: Option<&str>,
) -> Result<Vec<(String, String)>, ErrorData> {
    let mut sql = "SELECT path, content FROM files WHERE language = ?1 AND file_kind = 'indexed' AND content IS NOT NULL".to_owned();
    if pattern.is_some() {
        sql.push_str(" AND path GLOB ?2");
    }
    sql.push_str(" ORDER BY path ASC");
    let mut stmt = connection
        .prepare(&sql)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let glob = pattern
        .map(|p| {
            if p.contains(['*', '?']) {
                p.to_owned()
            } else {
                format!("*{p}*")
            }
        })
        .unwrap_or_default();
    let params: &[&dyn rusqlite::ToSql] = if pattern.is_some() {
        &[&language, &glob]
    } else {
        &[&language]
    };

    let mut rows = stmt
        .query(params)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    let mut result = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
    {
        let path: String = row
            .get(0)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let content: String = row
            .get(1)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        result.push((path, content));
    }
    Ok(result)
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
        (first_line.to_owned(), text.lines().count() > 1)
    } else {
        let mut end = max_bytes;
        while end > 0 && !first_line.is_char_boundary(end) {
            end -= 1;
        }
        (first_line[..end].to_owned(), true)
    }
}
