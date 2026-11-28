//! SQL statement validation, `PRAGMA` / `EXPLAIN PRAGMA` lexing, and
//! SQLite abort/error classification for the `query` tool.

use super::value::ensure_unique_column_names;
use crate::error_codes::{self, ErrorCode};
use rmcp::ErrorData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Which budget, if any, the progress handler recorded before the statement
/// stopped. Shared with `exec`, which installs the handler.
pub(super) const ABORT_NONE: u8 = 0;
pub(super) const ABORT_STEP_BUDGET: u8 = 1;
pub(super) const ABORT_WALL_CLOCK: u8 = 2;

/// Proves `sql` is exactly one statement and returns its text without the
/// terminating `;`, so it can sit inside the wrapper's parentheses.
///
/// SQLite's own parser decides: `Connection::prepare` compiles the first
/// statement and, when input remains after it, prepares that remainder and
/// fails with `Error::MultipleStatement` only if the remainder holds a real
/// statement (rusqlite 0.43.0). Whitespace, comments and
/// bare `;` after the first statement are therefore accepted, and a `;`
/// inside a literal, quoted identifier or comment never counts. No SQL is
/// lexed for the statement body. `expanded_sql` (no parameters are bound) is the statement text
/// SQLite consumed, which excludes any trailing comment after the `;`.
pub(super) fn single_statement_body(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    abort_reason: &Arc<AtomicU8>,
) -> Result<String, ErrorData> {
    // Several PRAGMAs take effect while being *prepared* (the heap limits,
    // `temp_store_directory`), and `prepare` also prepares every statement
    // after the first to detect `MultipleStatement`. So every statement's
    // first keyword is checked before the text reaches the parser. Every
    // other non-SELECT kind has no prepare-time effect and is refused by the
    // wrapper below.
    if any_statement_is_pragma(sql) {
        return Err(ErrorData::invalid_params(
            "sql was refused: `query` accepts a single read-only SELECT/WITH statement, \
             not PRAGMA (a PRAGMA can change process state while being prepared)",
            None,
        ));
    }
    let statement = tx.prepare(sql).map_err(|error| match error {
        rusqlite::Error::Empty => error_codes::invalid_params_with_code(
            "sql must contain a statement, not only comments",
            ErrorCode::InvalidInput,
        ),
        rusqlite::Error::MultipleStatement => ErrorData::invalid_params(
            "sql contains more than one statement; \
             only a single read-only statement is supported",
            None,
        ),
        other => describe_error(&other, abort_reason),
    })?;
    // Check the caller's own projection, before the wrapper turns it into a
    // subquery: SQLite renames the second `id` of a subquery to `id:1`, so
    // checking only the wrapped result would miss the duplicate and hand back
    // a renamed key the caller never asked for.
    let column_names: Vec<String> = statement.column_names().map(str::to_owned).collect();
    ensure_unique_column_names(&column_names)?;
    if !statement.readonly() {
        return Err(ErrorData::invalid_params(
            "sql was refused: `query` accepts a single read-only SELECT/WITH statement",
            None,
        ));
    }
    let text = statement.expanded_sql().ok_or_else(|| {
        crate::tools::context::internal_error(
            "reading expanded SQL",
            "SQLite returned no expanded SQL text",
        )
    })?;
    Ok(text.trim_end().trim_end_matches(';').to_owned())
}

/// The refusal for a statement that SQLite parses on its own but cannot run
/// as the wrapper's subquery: it is valid SQL of another kind. Only the
/// wrapper's syntax error is classified this way, because the bare statement
/// already compiled, so a syntax error can come only from the wrapping; any
/// other failure keeps [`describe_error`]'s text.
pub(super) fn refuse_wrapped(error: &rusqlite::Error, abort_reason: &Arc<AtomicU8>) -> ErrorData {
    if !matches!(error, rusqlite::Error::SqlInputError { .. }) {
        return describe_error(error, abort_reason);
    }
    ErrorData::invalid_params(
        format!(
            "sql was refused: `query` accepts a single read-only SELECT/WITH statement, and \
             this is another kind of statement (PRAGMA, EXPLAIN, a write, DDL or a \
             transaction command), so it cannot run as a subquery (SQLite: {error})"
        ),
        None,
    )
}

/// Converts an aborted-query error into a message naming which budget was
/// hit, rather than surfacing SQLite's generic "interrupted" text as if it
/// were always the step budget. SQLite's own error is kept in the message
/// and logged here, once.
pub(super) fn describe_error(error: &rusqlite::Error, abort_reason: &Arc<AtomicU8>) -> ErrorData {
    match abort_reason.load(Ordering::Relaxed) {
        ABORT_STEP_BUDGET => error_codes::invalid_params_with_code(
            format!("query exceeded its virtual-machine step budget: {error}"),
            ErrorCode::QueryBudgetExhausted,
        ),
        ABORT_WALL_CLOCK => error_codes::invalid_params_with_code(
            format!("query exceeded its wall-clock time budget: {error}"),
            ErrorCode::QueryBudgetExhausted,
        ),
        _ => ErrorData::invalid_params(error.to_string(), sqlite_error_payload(error)),
    }
}

/// True when any statement of `sql` (split at each `;` outside a literal,
/// quoted identifier or comment) starts with `PRAGMA`.
fn any_statement_is_pragma(sql: &str) -> bool {
    let mut rest = sql;
    loop {
        if first_keyword_is_pragma(rest) {
            return true;
        }
        match statement_end(rest).and_then(|end| rest.get(end..)?.get(1..)) {
            Some(after) => rest = after,
            None => return false,
        }
    }
}

/// Byte index of the first `;` in `sql` that ends a statement: one outside a
/// single-, double- or backtick-quoted string, a `[bracket]` identifier, a
/// `--` line comment and a `/* */` block comment. `None` when no statement
/// ends in `sql`. Every delimiter is ASCII, so the index is a char boundary.
fn statement_end(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut rest = bytes;
    while let Some((&byte, after)) = rest.split_first() {
        let (opener, closing): (usize, &[u8]) = match byte {
            b';' => return bytes.len().checked_sub(rest.len()),
            b'\'' => (1, b"'"),
            b'"' => (1, b"\""),
            b'`' => (1, b"`"),
            b'[' => (1, b"]"),
            b'-' if after.first() == Some(&b'-') => (2, b"\n"),
            b'/' if after.first() == Some(&b'*') => (2, b"*/"),
            _ => {
                rest = after;
                continue;
            }
        };
        let body = rest.get(opener..)?;
        let close = body
            .windows(closing.len())
            .position(|window| window == closing)?;
        rest = body.get(close..)?.get(closing.len()..)?;
    }
    None
}

/// True when `sql`'s first keyword (or the keyword after `EXPLAIN` /
/// `EXPLAIN QUERY PLAN`), after leading whitespace and comments, is `PRAGMA`
/// (case-insensitive).
fn first_keyword_is_pragma(sql: &str) -> bool {
    let mut rest = skip_leading_ws_and_comments(sql);
    let mut words = split_keywords(rest);
    match words.next() {
        Some(word) if word.eq_ignore_ascii_case("pragma") => true,
        Some(word) if word.eq_ignore_ascii_case("explain") => {
            let Some(tail) = rest.get(word.len()..) else {
                return false;
            };
            rest = skip_leading_ws_and_comments(tail);
            let mut after_explain = split_keywords(rest);
            match after_explain.next() {
                Some(next) if next.eq_ignore_ascii_case("pragma") => true,
                Some(next) if next.eq_ignore_ascii_case("query") => {
                    let Some(query_tail) = rest.get(next.len()..) else {
                        return false;
                    };
                    let after_query = skip_leading_ws_and_comments(query_tail);
                    let mut after_query_words = split_keywords(after_query);
                    let Some(plan) = after_query_words.next() else {
                        return false;
                    };
                    if !plan.eq_ignore_ascii_case("plan") {
                        return false;
                    }
                    let Some(plan_tail) = after_query.get(plan.len()..) else {
                        return false;
                    };
                    let after_plan = skip_leading_ws_and_comments(plan_tail);
                    split_keywords(after_plan)
                        .next()
                        .is_some_and(|kw| kw.eq_ignore_ascii_case("pragma"))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn split_keywords(sql: &str) -> impl Iterator<Item = &str> {
    sql.split(|character: char| !character.is_ascii_alphabetic())
}

fn skip_leading_ws_and_comments(sql: &str) -> &str {
    let mut rest = sql;
    loop {
        // SQLite treats ASCII vertical tab (0x0B) as whitespace; `is_ascii_whitespace` does not.
        rest = rest.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || character == '\u{000b}'
        });
        if let Some(comment) = rest.strip_prefix("--") {
            // A line comment ends at the newline; without one it runs to the
            // end of the statement, leaving no keyword.
            rest = match comment.split_once('\n') {
                Some((_, after)) => after,
                None => "",
            };
            continue;
        }
        if let Some(comment) = rest.strip_prefix("/*") {
            match comment.split_once("*/") {
                Some((_, after)) => {
                    rest = after;
                    continue;
                }
                None => return "",
            }
        }
        break;
    }
    rest
}

/// `sql_error` when SQLite itself failed to compile or run the statement;
/// `None` for a failure raised on this side of the boundary (an oversized
/// value, a type conversion), whose message already names its own fix.
fn sqlite_error_payload(error: &rusqlite::Error) -> Option<serde_json::Value> {
    match error {
        rusqlite::Error::SqliteFailure(..) | rusqlite::Error::SqlInputError { .. } => {
            Some(error_codes::payload(ErrorCode::SqlError))
        }
        _ => None,
    }
}
