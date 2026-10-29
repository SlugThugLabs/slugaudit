// slugaudit-line-exception: approved-by=slugthug; reason=the compiled leads statements and the reads that execute them, including the files_not_parsed count that qualifies them, form one cohesive read layer
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

use super::PARSER_FAILURE_PREDICATE;
use super::leads::{FileCount, LeadRow, MechanicCount};
use super::leads_covers::LanguagesRead;
use super::leads_mechanics::{MECHANICS, Mechanic};
use super::mechanic::MechanicSource;
use rmcp::ErrorData;
use rusqlite::{Transaction, params_from_iter};

/// Longest `detail` string returned per row. Callee text and comment lines
/// can be arbitrarily long; the caller can always read the file itself.
pub(super) const MAX_DETAIL_CHARS: usize = 200;

/// Clips a row's `detail` to `MAX_DETAIL_CHARS` characters on a char
/// boundary, setting `detail_clipped` only when characters were removed.
/// No shared truncation helper exists in `crate::util`.
pub(super) fn clip_detail(row: &mut LeadRow) {
    if row.detail.chars().count() <= MAX_DETAIL_CHARS {
        return;
    }
    row.detail = row.detail.chars().take(MAX_DETAIL_CHARS).collect();
    row.detail_clipped = true;
}

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

/// The row query for a mechanic, returning `(path, line, detail)` in the
/// mechanic's stable order. The only placeholder is the pattern, when present.
fn rows_sql(mechanic: &Mechanic, matched: bool) -> String {
    format!(
        "SELECT {}, {}, {} FROM {}{} ORDER BY {}",
        mechanic.path_expr(),
        mechanic.line_expr(),
        mechanic.detail_expr(),
        mechanic.read_from(),
        mechanic.where_clause(matched),
        mechanic.order_expr()
    )
}

/// Counts for every built-in mechanic, in `MECHANICS` order, under
/// `pattern`; `read` is the language sets under the same pattern.
pub(super) fn count_all(
    tx: &Transaction<'_>,
    pattern: Option<&str>,
    read: &LanguagesRead,
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
                source: MechanicSource::Builtin,
                note: None,
                count,
                files,
                covers: read.covers(mechanic.reads),
            })
        })
        .collect()
}

/// Indexed files under `pattern` with a detected language whose grammar did
/// not run or did not finish: the predicate `report` uses for its parser
/// failures (prose excluded), so the two tools cannot disagree, narrowed to
/// `language IS NOT NULL` — a file with no detected language (`.gitignore`,
/// `LICENSE`) has no grammar to run, so no mechanic could have read it, and
/// counting it made almost every project report a non-zero floor. `report`
/// still lists those files. See `LeadsResponse::files_not_parsed`.
pub(super) fn files_not_parsed(
    tx: &Transaction<'_>,
    pattern: Option<&str>,
) -> Result<i64, ErrorData> {
    let glob = compile_pattern(pattern);
    let filter = if glob.is_some() {
        " AND path GLOB ?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT count(*) FROM files WHERE {PARSER_FAILURE_PREDICATE} \
         AND language IS NOT NULL{filter}"
    );
    tx.query_row(&sql, params_from_iter(glob.iter()), |row| row.get(0))
        .map_err(|error| super::context::internal_error("counting files not parsed", error))
}

/// Streams every row of `mechanic` under `pattern`, in `order_expr` order,
/// into `on_row`.
///
/// `detail` is clipped to `MAX_DETAIL_CHARS` before the row is handed on, so
/// the stored row is exactly what the caller receives.
pub(super) fn stream_rows(
    tx: &Transaction<'_>,
    mechanic: &Mechanic,
    pattern: Option<&str>,
    on_row: &mut dyn FnMut(LeadRow) -> Result<(), ErrorData>,
) -> Result<(), ErrorData> {
    let glob = compile_pattern(pattern);
    let mut statement = tx
        .prepare(&rows_sql(mechanic, glob.is_some()))
        .map_err(|error| super::context::internal_error("preparing leads rows", error))?;
    let mapped = statement
        .query_map(params_from_iter(glob.iter()), |row| {
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
    for row in mapped {
        on_row(row.map_err(|error| super::context::internal_error("reading leads rows", error))?)?;
    }
    Ok(())
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
