//! Small SQL counting helpers shared across `report` section builders.

use rusqlite::Transaction;

pub(super) fn scalar_sql(conn: &Transaction<'_>, sql: &str) -> Result<i64, rusqlite::Error> {
    conn.query_row(sql, [], |row| row.get(0))
}

pub(super) fn count_where(
    conn: &Transaction<'_>,
    table: &str,
    filter: Option<&str>,
) -> Result<i64, rusqlite::Error> {
    match filter {
        Some(pred) => scalar_sql(conn, &format!("SELECT count(*) FROM {table} WHERE {pred}")),
        None => scalar_sql(conn, &format!("SELECT count(*) FROM {table}")),
    }
}

pub(super) fn grouped_column_counts<T>(
    conn: &Transaction<'_>,
    table: &str,
    col: &str,
    filter: Option<&str>,
    make: impl Fn(String, i64) -> T,
) -> Result<Vec<T>, rusqlite::Error> {
    let where_clause = match filter {
        Some(pred) => format!(" WHERE {pred}"),
        None => String::new(),
    };
    let sql =
        format!("SELECT {col}, count(*) FROM {table}{where_clause} GROUP BY {col} ORDER BY {col}");
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_map([], |row| Ok(make(row.get(0)?, row.get(1)?)))?
        .collect()
}
