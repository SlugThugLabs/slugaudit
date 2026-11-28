use super::*;
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::test_support::{create_project, setup_project, sync_project, write, write_file};
use crate::sync::{SourceSyncManager, SyncedProject};
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult, ensure_eq, ensure_ne};
use std::path::Path;
use tempfile::TempDir;

#[track_caller]
fn check_run(connection: &Connection, path: &str, avail: &str, outcome: &str) -> TestResult {
    ensure_eq(
        &stored_run(connection, path)?,
        &(avail.to_owned(), outcome.to_owned()),
        path,
    )
}

#[track_caller]
fn check_synced_runs(db_path: &Path, avail: &str, outcome: &str) -> TestResult<Connection> {
    let connection = open_read_write(db_path).check("open db")?;
    for path in ["a.rs", "b.rs"] {
        check_run(&connection, path, avail, outcome)?;
    }
    Ok(connection)
}

struct FailedDownloadFixture {
    connection: Connection,
    _db_dir: TempDir,
    project: TempDir,
    _guards: ParseTestGuards,
}

/// A project whose first publish had every grammar download fail, so its
/// files are stored `LoadFailed`, with content untouched afterwards.
fn project_with_failed_download() -> TestResult<FailedDownloadFixture> {
    let guards = reset_parse_guards();
    let project = tempfile::tempdir().check("project dir")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    write(project.path(), "a.rs", b"fn a() {}")?;
    {
        let _forced = forced_failure::force(|| PackError::Download("offline".into()));
        publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
            .check("publish with the download failing")?;
    }
    check_run(&connection, "a.rs", "LoadFailed", "NotAttempted")?;
    // The failed fetch closed the gate; these callers test the retry itself.
    test_controls::elapse_cooldown();
    Ok(FailedDownloadFixture {
        connection,
        _db_dir: db_dir,
        project,
        _guards: guards,
    })
}

#[test]
fn a_load_failed_file_is_reparsed_by_a_full_publish_with_unchanged_content() -> TestResult {
    let mut fx = project_with_failed_download()?;

    let report = publish(
        &mut fx.connection,
        fx.project.path(),
        "1.0",
        &NoopProgressSink,
    )
    .check("second publish with loading allowed")?;

    ensure_eq(&report.modified, &1, format!("{report:?}"))?;
    check_run(&fx.connection, "a.rs", "Available", "Succeeded")?;
    Ok(())
}

#[test]
fn a_succeeded_file_with_unchanged_content_is_still_skipped_by_the_next_sync() -> TestResult {
    let (project, _db_dir, mut connection, revision) = setup_project()?;

    let report = publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
        .check("second publish")?;

    ensure_eq(&report.revision_id, &revision, "no new revision")?;
    ensure_eq(
        &(report.added, report.modified),
        &(0, 0),
        "(report.added, report.modified) == (0, 0)",
    )?;
    ensure_eq(&report.unchanged, &2, "report.unchanged == 2")?;
    Ok(())
}

fn stored_reason(connection: &Connection, path: &str) -> TestResult<String> {
    connection
        .query_row(
            "SELECT parse_error_reason FROM files WHERE path = ?1",
            [path],
            |row| row.get(0),
        )
        .check("stored reason")
}

/// A project with two Rust files, synced through the manager while every
/// grammar fetch fails. Returns the guards, the manager, the project and the
/// synced state.
fn synced_with_failing_fetch()
-> TestResult<(ParseTestGuards, SourceSyncManager, TempDir, SyncedProject)> {
    let guards = reset_parse_guards();
    test_controls::set_grammar_cached(false);
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "a.rs", b"fn a() {}")?;
    write_file(&project, "b.rs", b"fn b() {}")?;
    let synced = {
        let _forced = forced_failure::force(|| PackError::Download("offline".into()));
        sync_project(&manager, &project)?
    };
    Ok((guards, manager, project, synced))
}

#[test]
fn a_second_file_in_the_same_sync_does_not_reach_the_loader_after_a_fetch_failure() -> TestResult {
    let (_guards, _manager, _project, synced) = synced_with_failing_fetch()?;

    ensure_eq(&forced_failure::calls(), &1, "one failed fetch, then none")?;
    let connection = check_synced_runs(&synced.database_path, "LoadFailed", "NotAttempted")?;
    let reasons = [
        stored_reason(&connection, "a.rs")?,
        stored_reason(&connection, "b.rs")?,
    ];
    ensure_eq(
        &reasons.iter().filter(|r| **r == COOLDOWN_REASON).count(),
        &1,
        format!("exactly the file after the failure carries the cool-down reason: {reasons:?}"),
    )?;
    Ok(())
}

#[test]
fn an_unchanged_load_failed_file_publishes_no_revision_during_the_cooldown() -> TestResult {
    let (_guards, manager, project, first) = synced_with_failing_fetch()?;
    let calls_after_first_sync = forced_failure::calls();

    let second = sync_project(&manager, &project)?;

    ensure_eq(&second.revision_id, &first.revision_id, "no new revision")?;
    ensure_eq(
        &forced_failure::calls(),
        &calls_after_first_sync,
        "no loader call",
    )?;
    Ok(())
}

#[test]
fn a_load_failed_file_is_retried_and_parses_once_the_cooldown_has_elapsed() -> TestResult {
    let (_guards, manager, project, first) = synced_with_failing_fetch()?;
    test_controls::elapse_cooldown();

    let second = sync_project(&manager, &project)?;

    ensure_ne(
        &second.revision_id,
        &first.revision_id,
        "the retry publishes",
    )?;
    check_synced_runs(&second.database_path, "Available", "Succeeded")?;
    Ok(())
}
