//! The statements each `leads` family compiles to, and the reads that
//! execute them.
//!
//! Split from `leads.rs` so the tool contract — request/response shapes,
//! paging, and the transaction envelope — lives in one place, and split from
//! `leads_mechanics.rs` so the catalogue and each family's SQL shape stay a
//! declarative table. Nothing here draws a conclusion about what a row
//! means.
//!
//! Every read accepts the same optional `pattern`; it is compiled once by
//! [`compile_pattern`] and bound as a SQL parameter, never interpolated.

use super::leads::{FileCount, LeadRow, MechanicCount, PageLimit, clip_detail};
use super::leads_mechanics::{MECHANICS, Mechanic};
use crate::util::serialized_len;
use rmcp::ErrorData;
use rusqlite::types::Value;
use rusqlite::{Transaction, params_from_iter};

/// Compiles the caller's `pattern` exactly the way `structure` does — a
/// GLOB over the project-relative path, with a pattern containing no `*` or
/// `?` treated as a substring match. The result is always bound as a
/// parameter, never spliced into SQL.
pub(super) fn compile_pattern(pattern: Option<&str>) -> Option<String> {
    pattern.map(|pattern| {
        if pattern.contains(['*', '?']) {
            pattern.to_owned()
        } else {
            format!("*{pattern}*")
        }
    })
}

/// `SELECT count(*), count(DISTINCT <file>)` for a mechanic's family, under
/// the same `pattern` the row read uses. Counts are always complete, never
/// paged, so they never carry `LIMIT` — `?` is the pattern when one is
/// supplied.
fn count_sql(mechanic: &Mechanic, matched: bool) -> String {
    format!(
        "SELECT count(*), count(DISTINCT {}) FROM {}{}",
        mechanic.file_key_expr(),
        mechanic.read_from(),
        mechanic.where_clause(matched)
    )
}

/// The paged row query for a mechanic, returning `(path, line, detail)`.
/// Placeholders bind in textual order: the pattern (when present), then the
/// limit, then the offset.
fn rows_sql(mechanic: &Mechanic, matched: bool) -> String {
    format!(
        "SELECT {}, {}, {} FROM {}{} ORDER BY {} LIMIT ? OFFSET ?",
        mechanic.path_expr(),
        mechanic.line_expr(),
        mechanic.detail_expr(),
        mechanic.read_from(),
        mechanic.where_clause(matched),
        mechanic.order_expr()
    )
}

/// Distinct files represented in the first `LIMIT` rows of the same ordered
/// result `rows_sql` walks — the cumulative file count after a page.
fn covered_sql(mechanic: &Mechanic, matched: bool) -> String {
    format!(
        "SELECT count(DISTINCT path) FROM (SELECT {} AS path FROM {}{} ORDER BY {} LIMIT ?)",
        mechanic.path_expr(),
        mechanic.read_from(),
        mechanic.where_clause(matched),
        mechanic.order_expr()
    )
}

/// Binds a `usize` as SQLite's 64-bit integer; `None` becomes the largest
/// value SQLite accepts, i.e. "no limit".
pub(super) fn int_bind(value: Option<usize>) -> i64 {
    value.map_or(i64::MAX, |value| i64::try_from(value).unwrap_or(i64::MAX))
}

/// Counts for every mechanic, in `MECHANICS` order, under `pattern`.
pub(super) fn count_all(
    tx: &Transaction<'_>,
    pattern: Option<&str>,
) -> Result<Vec<MechanicCount>, ErrorData> {
    let glob = compile_pattern(pattern);
    MECHANICS
        .iter()
        .map(|mechanic| {
            let (count, files): (i64, i64) = match glob.as_deref() {
                Some(glob) => tx.query_row(&count_sql(mechanic, true), [glob], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                }),
                None => tx.query_row(&count_sql(mechanic, false), [], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                }),
            }
            .map_err(|error| super::context::internal_error("counting leads", error))?;
            Ok(MechanicCount {
                mechanic: mechanic.id.to_owned(),
                count,
                files,
                covers: mechanic.covers,
            })
        })
        .collect()
}

/// One page of rows: the rows, their serialized JSON size, and what stopped
/// the page. Generic over the row shape so the full reader and the compact
/// reader charge the same budget the same way.
pub(super) struct Page<R> {
    pub(super) rows: Vec<R>,
    /// Sum of each returned row's serialized JSON length — the same measure
    /// `query` applies, so the two tools' byte budgets are comparable.
    pub(super) bytes: usize,
    pub(super) limited_by: PageLimit,
}

/// Charges mapped rows against the caller's `limit` and the response byte
/// budget. The caller fetched one row past `limit` when one was set, so
/// `LimitedBy::Limit` can be told apart from exhaustion; that row is never
/// kept. The first row is always kept even when it alone exceeds
/// `max_bytes`; stopping at zero rows would make the budget
/// indistinguishable from an empty result set.
pub(super) fn fill_page<R: serde::Serialize>(
    mapped: impl Iterator<Item = rusqlite::Result<R>>,
    limit: Option<usize>,
    max_bytes: usize,
    context: &'static str,
) -> Result<Page<R>, ErrorData> {
    let mut page = Page {
        rows: Vec::new(),
        bytes: 0,
        limited_by: PageLimit::None,
    };
    for row in mapped {
        if limit.is_some_and(|limit| page.rows.len() >= limit) {
            page.limited_by = PageLimit::Limit;
            break;
        }
        let row = row.map_err(|error| super::context::internal_error(context, error))?;
        let row_bytes =
            serialized_len(&row).map_err(|error| super::context::internal_error(context, error))?;
        if !page.rows.is_empty() && page.bytes.saturating_add(row_bytes) > max_bytes {
            page.limited_by = PageLimit::Bytes;
            break;
        }
        page.bytes = page.bytes.saturating_add(row_bytes);
        page.rows.push(row);
    }
    Ok(page)
}

/// Reads one page of `mechanic`'s rows, in `order_expr` order.
///
/// `detail` is clipped to `MAX_DETAIL_CHARS` before the row is measured, so
/// the budget is charged for exactly the bytes the caller receives.
pub(super) fn rows_for(
    tx: &Transaction<'_>,
    mechanic: &Mechanic,
    pattern: Option<&str>,
    limit: Option<usize>,
    offset: usize,
    max_bytes: usize,
) -> Result<Page<LeadRow>, ErrorData> {
    let glob = compile_pattern(pattern);
    // One row past the caller's limit is fetched only to learn whether more
    // exist (so `limited_by` can say `limit` rather than nothing); it is
    // never returned.
    let fetch = limit.map(|rows| rows.saturating_add(1));
    let mut binds = Vec::new();
    if let Some(glob) = glob.as_deref() {
        binds.push(Value::Text(glob.to_owned()));
    }
    binds.push(Value::Integer(int_bind(fetch)));
    binds.push(Value::Integer(int_bind(Some(offset))));

    let mut statement = tx
        .prepare(&rows_sql(mechanic, glob.is_some()))
        .map_err(|error| super::context::internal_error("preparing leads rows", error))?;
    let mapped = statement
        .query_map(params_from_iter(binds), |row| {
            let mut row = LeadRow {
                file: row.get(0)?,
                line: row.get(1)?,
                detail: row.get(2)?,
                detail_clipped: false,
            };
            clip_detail(&mut row);
            Ok(row)
        })
        .map_err(|error| super::context::internal_error("reading leads rows", error))?;
    fill_page(mapped, limit, max_bytes, "reading leads rows")
}

/// Distinct files represented in the first `through` rows of the mechanic's
/// ordered result. `through = 0` is zero by construction.
pub(super) fn files_covered(
    tx: &Transaction<'_>,
    mechanic: &Mechanic,
    pattern: Option<&str>,
    through: usize,
) -> Result<i64, ErrorData> {
    let glob = compile_pattern(pattern);
    let mut binds = Vec::new();
    if let Some(glob) = glob.as_deref() {
        binds.push(Value::Text(glob.to_owned()));
    }
    binds.push(Value::Integer(int_bind(Some(through))));
    tx.query_row(
        &covered_sql(mechanic, glob.is_some()),
        params_from_iter(binds),
        |row| row.get(0),
    )
    .map_err(|error| super::context::internal_error("counting covered files", error))
}

/// The per-file histogram of one page's files, in first-appearance order.
/// Pages come back ordered by path, so equal paths are adjacent.
pub(super) fn page_file_counts<'a>(files: impl Iterator<Item = &'a str>) -> Vec<FileCount> {
    let mut counts: Vec<FileCount> = Vec::new();
    for file in files {
        match counts.last_mut() {
            Some(last) if last.file == file => last.count += 1,
            _ => counts.push(FileCount {
                file: file.to_owned(),
                count: 1,
            }),
        }
    }
    counts
}
