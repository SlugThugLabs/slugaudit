//! SQL scalar functions registered on SlugAudit's own connections:
//! `line_count(text)` and `line_range(text, start, end)`.
//!
//! They exist so an agent can read part of a large `files.content` without
//! pulling the whole file through a result: `line_range(content, 400, 600)`.
//!
//! Line model (1-based): lines are separated by `\n`. A final line without a
//! terminator still counts, and a terminator does not start a further empty
//! line, so `"a\nb"` and `"a\nb\n"` both have 2 lines and `""` has 0. `\r` is
//! not a separator: a `\r\n` file keeps its `\r` bytes inside the text that
//! `line_range` returns between lines.
//!
//! Both are a single forward scan of the bytes (`line_range` stops at line
//! `end`), so a call is O(bytes up to `end`), never per-line rescans.

use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Error, Result};

/// Registers both functions as deterministic, UTF-8 scalar functions.
pub(super) fn register(connection: &Connection) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    connection.create_scalar_function("line_count", 1, flags, |context| {
        let Some(text) = text_arg(context, 0, "line_count")? else {
            return Ok(None);
        };
        let count = i64::try_from(count_lines(text)).map_err(|error| {
            user_error(format!(
                "line_count: count does not fit an integer: {error}"
            ))
        })?;
        Ok(Some(count))
    })?;
    connection.create_scalar_function("line_range", 3, flags, |context| {
        let text = text_arg(context, 0, "line_range")?;
        let start = integer_arg(context, 1, "line_range", "start")?;
        let end = integer_arg(context, 2, "line_range", "end")?;
        let (Some(text), Some(start), Some(end)) = (text, start, end) else {
            return Ok(None);
        };
        if start < 1 {
            return Err(user_error(format!(
                "line_range: start must be at least 1 (lines are 1-based), got {start}"
            )));
        }
        if end < start {
            return Err(user_error(format!(
                "line_range: end ({end}) must not be less than start ({start})"
            )));
        }
        Ok(Some(slice_lines(text, start, end).to_owned()))
    })?;
    Ok(())
}

fn user_error(message: String) -> Error {
    Error::UserFunctionError(message.into())
}

/// A TEXT argument, `None` for NULL; any other storage class is an error
/// rather than a silent coercion.
fn text_arg<'a>(context: &'a Context<'_>, index: usize, function: &str) -> Result<Option<&'a str>> {
    match context.get_raw(index) {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map(Some)
            .map_err(|error| user_error(format!("{function}: text is not valid UTF-8: {error}"))),
        other => Err(user_error(format!(
            "{function}: the first argument must be TEXT, got {}",
            other.data_type()
        ))),
    }
}

/// An INTEGER argument, `None` for NULL; a REAL, TEXT or BLOB is an error.
fn integer_arg(
    context: &Context<'_>,
    index: usize,
    function: &str,
    name: &str,
) -> Result<Option<i64>> {
    match context.get_raw(index) {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(value) => Ok(Some(value)),
        other => Err(user_error(format!(
            "{function}: {name} must be an INTEGER, got {}",
            other.data_type()
        ))),
    }
}

/// Number of lines under the model in the module docs.
fn count_lines(text: &str) -> usize {
    let bytes = text.as_bytes();
    let terminators = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated_last = bytes.last().is_some_and(|byte| *byte != b'\n');
    terminators + usize::from(unterminated_last)
}

fn next_newline(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| from + offset)
}

/// Lines `start..=end` (1-based, `1 <= start <= end`) exactly as in `text`,
/// without the terminator of the last returned line (`\n` or `\r\n`).
/// Empty when `start` is past the last line; stops at EOF when `end` is.
fn slice_lines(text: &str, start: i64, end: i64) -> &str {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    for _ in 1..start {
        match next_newline(bytes, cursor) {
            Some(newline) => cursor = newline + 1,
            None => return "",
        }
    }
    if cursor >= bytes.len() {
        return "";
    }
    let begin = cursor;
    let mut stop = bytes.len();
    for _ in start..=end {
        match next_newline(bytes, cursor) {
            Some(newline) => {
                cursor = newline + 1;
                stop = cursor;
            }
            None => {
                stop = bytes.len();
                break;
            }
        }
    }
    // `begin` and `stop` sit just after a `\n` or at the ends, so both are
    // char boundaries.
    let slice = &text[begin..stop];
    match slice.strip_suffix('\n') {
        Some(line) => line.strip_suffix('\r').unwrap_or(line),
        None => slice,
    }
}

#[cfg(test)]
#[path = "sql_functions_tests.rs"]
mod tests;
