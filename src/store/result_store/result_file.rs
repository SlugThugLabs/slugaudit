//! One result file: created private, opened, unlinked at once (the open
//! descriptor is the only reference), pragmas applied and read back, schema
//! created.

use super::error::{PragmaMismatch, ResultStoreError};
use super::platform;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use uuid::Uuid;

/// Most result columns: SQLite allows 2000 per table, two are ours.
pub(super) const MAX_COLUMNS: usize = 1000;

/// The column list `c0, c1, ...` for `count` columns (integers only, never
/// caller text).
pub(super) fn column_list(count: usize) -> String {
    (0..count)
        .map(|index| format!("c{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Creates `<dir>/<session>-<result>.db`, opens it, unlinks it, applies the
/// pragmas. On any failure the file is removed if it still exists.
pub(super) fn create(
    dir: &Path,
    session: Uuid,
    result: Uuid,
) -> Result<Connection, ResultStoreError> {
    let path = dir.join(format!(
        "{}-{}.db",
        session.hyphenated(),
        result.hyphenated()
    ));
    drop(
        platform::create_private_file(&path)
            .map_err(|error| ResultStoreError::unavailable("create a result file", error))?,
    );
    let opened = open_unlinked(&path);
    if opened.is_err() {
        remove_leftover(&path);
    }
    opened
}

fn open_unlinked(path: &Path) -> Result<Connection, ResultStoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| ResultStoreError::from_sqlite("open a result file", error))?;
    std::fs::remove_file(path)
        .map_err(|error| ResultStoreError::unavailable("unlink a result file", error))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// A file that survived a failed open; the startup sweep is the backstop.
fn remove_leftover(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(error = %error, "could not remove a failed result file; the startup sweep will");
        }
    }
}

/// Sets and reads back the result pragmas; a pragma that did not take fails
/// closed.
fn apply_pragmas(conn: &Connection) -> Result<(), ResultStoreError> {
    let sql = |error| ResultStoreError::from_sqlite("apply result pragmas", error);
    for (pragma, expected) in [("journal_mode", "off"), ("locking_mode", "exclusive")] {
        let actual: String = conn
            .query_row(&format!("PRAGMA {pragma}={expected}"), [], |row| row.get(0))
            .map_err(sql)?;
        require(pragma, expected, &actual.to_lowercase())?;
    }
    conn.execute_batch("PRAGMA synchronous=OFF").map_err(sql)?;
    let synchronous: i64 = conn
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .map_err(sql)?;
    require("synchronous", "0", &synchronous.to_string())
}

fn require(
    pragma: &'static str,
    expected: &'static str,
    actual: &str,
) -> Result<(), ResultStoreError> {
    if actual == expected {
        return Ok(());
    }
    Err(ResultStoreError::unavailable(
        "a result pragma did not take effect",
        PragmaMismatch {
            pragma,
            expected,
            actual: actual.to_owned(),
        },
    ))
}

/// Creates `meta` and `rows` with `columns` untyped value columns.
pub(super) fn create_schema(conn: &Connection, columns: usize) -> Result<(), ResultStoreError> {
    let sql = format!(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value);\
         CREATE TABLE rows (ord INTEGER PRIMARY KEY, nonfinite TEXT, {});",
        column_list(columns)
    );
    conn.execute_batch(&sql)
        .map_err(|error| ResultStoreError::from_sqlite("create the result schema", error))
}

/// The file's size: `page_count * page_size`.
pub(super) fn measure_bytes(conn: &Connection) -> Result<u64, ResultStoreError> {
    let read = |name: &str| -> Result<u64, ResultStoreError> {
        conn.pragma_query_value(None, name, |row| row.get::<_, u64>(0))
            .map_err(|error| ResultStoreError::from_sqlite("measure a result file", error))
    };
    Ok(read("page_count")?.saturating_mul(read("page_size")?))
}
