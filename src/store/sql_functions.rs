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

/// Largest TEXT value the line functions accept (`SQLITE_MAX_VALUE_BYTES`), so
/// `line_count(content)` and `line_range(content, ...)` work on any indexed
/// file stored under `Audit` or `Adaptive` profiles.
const MAX_LINE_FUNCTION_INPUT_BYTES: usize = 1_000_000_000;

/// Registers both functions as deterministic, UTF-8 scalar functions.
pub(super) fn register(connection: &Connection) -> Result<()> {
    register_with_cap(connection, MAX_LINE_FUNCTION_INPUT_BYTES)
}

fn register_with_cap(connection: &Connection, max_input_bytes: usize) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    connection.create_scalar_function("line_count", 1, flags, move |context| {
        let Some(text) = text_arg(context, 0, "line_count", max_input_bytes)? else {
            return Ok(None);
        };
        let count = i64::try_from(count_lines(text)).map_err(|error| {
            user_error(format!(
                "line_count: count does not fit an integer: {error}"
            ))
        })?;
        Ok(Some(count))
    })?;
    connection.create_scalar_function("line_range", 3, flags, move |context| {
        let text = text_arg(context, 0, "line_range", max_input_bytes)?;
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
    connection.create_scalar_function("slugaudit_path_match", 2, flags, move |context| {
        let Some(path) = text_arg(context, 0, "slugaudit_path_match", max_input_bytes)? else {
            return Ok(false);
        };
        let Some(pattern) = text_arg(context, 1, "slugaudit_path_match", max_input_bytes)? else {
            return Ok(false);
        };
        Ok(crate::util::search_path_matches(path, pattern))
    })?;
    #[cfg(test)]
    register_golden_nan(connection)?;
    Ok(())
}

/// Test-only `golden_nan()`: returns a REAL NaN, the one route the golden
/// test has to ask whether a NaN can reach a query result at all.
#[cfg(test)]
fn register_golden_nan(connection: &Connection) -> Result<()> {
    connection.create_scalar_function(
        "golden_nan",
        0,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |_| Ok(f64::NAN),
    )
}

fn user_error(message: String) -> Error {
    Error::UserFunctionError(message.into())
}

/// A TEXT argument, `None` for NULL; any other storage class is an error
/// rather than a silent coercion.
fn text_arg<'a>(
    context: &'a Context<'_>,
    index: usize,
    function: &str,
    max_input_bytes: usize,
) -> Result<Option<&'a str>> {
    match context.get_raw(index) {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => {
            // Checked on the raw bytes, before `from_utf8` and before either
            // caller scans or copies the value.
            if bytes.len() > max_input_bytes {
                return Err(user_error(format!(
                    "{function}: TEXT argument exceeds the {max_input_bytes}-byte \
                     input cap; apply substr() to the value first"
                )));
            }
            std::str::from_utf8(bytes).map(Some).map_err(|error| {
                user_error(format!("{function}: text is not valid UTF-8: {error}"))
            })
        }
        other => Err(user_error(format!(
            "{function}: {} argument must be TEXT, got {}",
            ordinal_arg(index),
            other.data_type()
        ))),
    }
}

fn ordinal_arg(index: usize) -> std::borrow::Cow<'static, str> {
    match index {
        0 => "the first".into(),
        1 => "the second".into(),
        2 => "the third".into(),
        _ => match index.checked_add(1) {
            Some(pos) => {
                let suffix = match (pos.checked_rem(100), pos.checked_rem(10)) {
                    (Some(11..=13), _) => "th",
                    (_, Some(1)) => "st",
                    (_, Some(2)) => "nd",
                    (_, Some(3)) => "rd",
                    _ => "th",
                };
                format!("the {pos}{suffix}").into()
            }
            None => "the".into(),
        },
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
    text.split_inclusive('\n').count()
}

/// The offset just past the next `\n` at or after `from`, or `None` when
/// there is none (or `from` is past the end).
fn after_next_newline(bytes: &[u8], from: usize) -> Option<usize> {
    let offset = bytes.get(from..)?.iter().position(|byte| *byte == b'\n')?;
    from.checked_add(offset)?.checked_add(1)
}

/// Lines `start..=end` (1-based, `1 <= start <= end`) exactly as in `text`,
/// without the terminator of the last returned line (`\n` or `\r\n`).
/// Empty when `start` is past the last line; stops at EOF when `end` is.
fn slice_lines(text: &str, start: i64, end: i64) -> &str {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    for _ in 1..start {
        match after_next_newline(bytes, cursor) {
            Some(after) => cursor = after,
            None => return "",
        }
    }
    if cursor >= bytes.len() {
        return "";
    }
    let begin = cursor;
    let mut stop = bytes.len();
    for _ in start..=end {
        if let Some(after) = after_next_newline(bytes, cursor) {
            cursor = after;
            stop = cursor;
        } else {
            stop = bytes.len();
            break;
        }
    }
    // `begin` and `stop` sit just after a `\n` or at the ends, so both are
    // char boundaries.
    let Some(slice) = text.get(begin..stop) else {
        return "";
    };
    if let Some(line) = slice.strip_suffix('\n') {
        match line.strip_suffix('\r') {
            Some(without_return) => without_return,
            None => line,
        }
    } else {
        slice
    }
}

#[cfg(test)]
#[path = "sql_functions_tests.rs"]
mod tests;
