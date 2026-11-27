//! SQL statements and compact `UNION ALL` enumeration for `leads`.

use super::coverage::LanguagesRead;
use super::mechanics::{MECHANICS, Mechanic};
use super::{CompactLeadRow, LeadRow, MechanicCount, clip_detail};
use crate::tools::context::internal_error;
use crate::tools::mechanic::MechanicSource;
use rmcp::ErrorData;
use rusqlite::types::Value;
use rusqlite::{Row, Transaction, params_from_iter};

fn count_sql(mechanic: &Mechanic, matched: bool) -> String {
    let sql = mechanic.sql();
    format!(
        "SELECT count(*), count(DISTINCT {}) FROM {}{}",
        sql.file_key,
        sql.from,
        mechanic.where_clause(matched)
    )
}

fn rows_sql(mechanic: &Mechanic, matched: bool) -> String {
    let sql = mechanic.sql();
    format!(
        "SELECT {}, {}, {} FROM {}{} ORDER BY {}",
        sql.path,
        sql.line,
        mechanic.detail_expr(),
        sql.from,
        mechanic.where_clause(matched),
        sql.order
    )
}

pub(super) fn count_all(
    tx: &Transaction<'_>,
    glob: Option<&str>,
    read: &LanguagesRead,
) -> Result<Vec<MechanicCount>, ErrorData> {
    MECHANICS
        .iter()
        .map(|mechanic| {
            let sql = count_sql(mechanic, glob.is_some());
            let (count, files): (i64, i64) = tx
                .query_row(&sql, params_from_iter(glob.iter()), |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(|error| internal_error("counting leads", error))?;
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

fn stream_sql<T, P>(
    tx: &Transaction<'_>,
    sql: &str,
    params: P,
    op: &'static str,
    decode: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    on_row: &mut dyn FnMut(T) -> Result<(), ErrorData>,
) -> Result<(), ErrorData>
where
    P: rusqlite::Params,
{
    let mut stmt = tx.prepare(sql).map_err(|error| internal_error(op, error))?;
    let mapped = stmt
        .query_map(params, decode)
        .map_err(|error| internal_error(op, error))?;
    for item in mapped {
        on_row(item.map_err(|error| internal_error(op, error))?)?;
    }
    Ok(())
}

pub(super) fn stream_rows(
    tx: &Transaction<'_>,
    mechanic: &Mechanic,
    glob: Option<&str>,
    on_row: &mut dyn FnMut(LeadRow) -> Result<(), ErrorData>,
) -> Result<(), ErrorData> {
    stream_sql(
        tx,
        &rows_sql(mechanic, glob.is_some()),
        params_from_iter(glob.iter()),
        "reading leads rows",
        |row| {
            let mut lead = LeadRow {
                file: row.get(0)?,
                line: row.get(1)?,
                detail: row.get(2)?,
                detail_clipped: false,
            };
            clip_detail(&mut lead);
            Ok(lead)
        },
        on_row,
    )
}

pub(super) fn compact_scope(selected: Option<&Mechanic>) -> Vec<&Mechanic> {
    match selected {
        Some(mechanic) => vec![mechanic],
        None => MECHANICS.iter().collect(),
    }
}

pub(super) fn stream_compact(
    tx: &Transaction<'_>,
    scope: &[&Mechanic],
    glob: Option<&str>,
    on_row: &mut dyn FnMut(CompactLeadRow) -> Result<(), ErrorData>,
) -> Result<(), ErrorData> {
    let branches = scope
        .iter()
        .enumerate()
        .map(|(ord, m)| {
            let s = m.sql();
            format!(
                "SELECT {ord} AS ord, '{}' AS mechanic, {} AS path, {} AS line, {} AS rid FROM {}{}",
                m.id,
                s.path,
                s.line,
                s.row_id,
                s.from,
                m.where_clause(glob.is_some())
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let sql =
        format!("SELECT mechanic, path, line FROM ({branches}) ORDER BY ord, path, line, rid");
    let binds = match glob {
        Some(g) => vec![Value::Text(g.to_owned()); scope.len()],
        None => Vec::new(),
    };
    stream_sql(
        tx,
        &sql,
        params_from_iter(binds),
        "reading compact leads rows",
        |row| {
            Ok(CompactLeadRow {
                mechanic: row.get(0)?,
                file: row.get(1)?,
                line: row.get(2)?,
            })
        },
        on_row,
    )
}
