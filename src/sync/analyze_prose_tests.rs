use super::*;
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::test_support::write;
use rusqlite::Connection;
use slugaudit_test_check::Check;
use tempfile::TempDir;

/// Publishes `files` (path, content) into a fresh project and returns the
/// open database; the project and database directories live as long as it.
fn published(files: &[(&str, &str)]) -> TestResult<(TempDir, TempDir, Connection)> {
    let project = tempfile::tempdir().check("project dir")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    for (path, content) in files {
        write(project.path(), path, content.as_bytes())?;
    }
    publish(&mut connection, project.path(), "1.0", &NoopProgressSink).check("publish")?;
    Ok((project, db_dir, connection))
}

fn count(connection: &Connection, sql: &str) -> TestResult<i64> {
    connection
        .query_row(sql, [], |row| row.get(0))
        .check("count query")
}

#[test]
fn prose_files_yield_no_diagnostics_and_stay_queryable() -> TestResult {
    let (_project, _db, connection) = published(&[
        ("README.md", "# T\n\n```rust\nfn unterminated( {\n"),
        ("notes.txt", "needle in plain text\n"),
    ])?;

    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM evidence WHERE kind = 'Diagnostic'",
        )?,
        &0,
        "count( &connection, \"SELECT COUNT(*) FROM evidence WHERE kind = 'Diagnostic'\" )? == 0",
    )?;
    ensure_eq(
        &count(&connection, "SELECT COUNT(*) FROM evidence")?,
        &0,
        "count(&connection, \"SELECT COUNT(*) FROM evidence\")? == 0",
    )?;
    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE content LIKE '%unterminated(%' \
             OR content LIKE '%needle%'",
        )?,
        &2,
        "count( &connection, \"SELECT COUNT(*) FROM files WHERE content LIKE '%unterminated(%' \\ OR content LIKE '%needle%'\" )? == 2",
    )?;
    ensure_eq(
        &stored_run(&connection, "notes.txt")?,
        &("Unavailable".to_owned(), "NotAttempted".to_owned()),
        "stored_run(&connection, \"notes.txt\")? == (\"Unavailable\".to_owned(), \"NotAttempted\".to_owned())",
    )?;
    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE language = 'prose' AND language_detected = 1 \
             AND extraction_completeness = 'ContentOnly'",
        )?,
        &2,
        "count( &connection, \"SELECT COUNT(*) FROM files WHERE language = 'prose' AND language_detected = 1 \\ AND extraction_completeness = 'ContentOnly'\" )? == 2",
    )?;
    Ok(())
}

#[test]
fn an_ignore_file_yields_no_diagnostics_and_never_reaches_the_loader() -> TestResult {
    let before = forced_failure::calls();

    let (_project, _db, connection) = published(&[
        (".gitignore", "target/\n.planning/slugaudit/\n"),
        ("lib.rs", "pub fn a() {}\n"),
    ])?;

    ensure_eq(
        &forced_failure::calls(),
        &(before + 1),
        "only lib.rs was parsed",
    )?;
    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = '.gitignore'",
        )?,
        &0,
        "count( &connection, \"SELECT COUNT(*) FROM evidence e JOIN files f ON f.id = e.file_id \\ WHERE f.path = '.gitignore'\" )? == 0",
    )?;
    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE path = '.gitignore' AND language = 'prose' \
             AND extraction_completeness = 'ContentOnly'",
        )?,
        &1,
        "count( &connection, \"SELECT COUNT(*) FROM files WHERE path = '.gitignore' AND language = 'prose' \\ AND extraction_completeness = 'ContentOnly'\" )? == 1",
    )?;
    Ok(())
}

#[test]
fn indexing_only_prose_files_never_reaches_the_loader() -> TestResult {
    let before = forced_failure::calls();

    let _published = published(&[("a.md", "# a\n"), ("b.txt", "b\n"), ("c.rst", "c\n====\n")])?;

    ensure_eq(
        &forced_failure::calls(),
        &before,
        "prose must not load a grammar",
    )?;
    Ok(())
}

#[test]
fn a_yaml_tab_indentation_error_yields_one_diagnostic_row() -> TestResult {
    let (_project, _db, connection) = published(&[("ci.yaml", "a:\n\t- b\n")])?;

    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM evidence WHERE kind = 'Diagnostic'",
        )?,
        &1,
        "count( &connection, \"SELECT COUNT(*) FROM evidence WHERE kind = 'Diagnostic'\" )? == 1",
    )?;
    Ok(())
}
