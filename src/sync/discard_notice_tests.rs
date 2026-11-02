use crate::store::test_capture::capture_warns;
use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};
use slugaudit_test_check::{Check, TestResult};

fn database(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning/slugaudit/project.db")
}

fn synced_project() -> TestResult<(SourceSyncManager, tempfile::TempDir)> {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn a() {}\n")?;
    sync_project(&manager, &project)?;
    Ok((manager, project))
}

#[test]
fn discard_surface_schema_mismatch_reports_the_cause_and_logs_once() -> TestResult {
    let (manager, project) = synced_project()?;
    {
        let connection = rusqlite::Connection::open(database(&project)).check("raw open")?;
        connection
            .execute_batch("PRAGMA user_version = 999;")
            .check("foreign schema version")?;
    }

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));
    let synced = synced?;

    let notice = synced.discard.check("the rebuild is reported")?;
    assert!(
        notice.cause.contains("corrupt or outdated"),
        "cause was {}",
        notice.cause
    );
    let warns: Vec<&str> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warns.len(), 1, "one WARN for one discard: {logs}");
    assert!(warns[0].contains("error="), "the cause is logged: {logs}");
    Ok(())
}

#[test]
fn discard_surface_corrupt_database_reports_the_cause() -> TestResult {
    let (manager, project) = synced_project()?;
    std::fs::write(database(&project), b"this is not a sqlite database").check("corrupt db")?;

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));
    let synced = synced?;

    let notice = synced.discard.check("the rebuild is reported")?;
    assert!(
        notice.cause.contains("corrupt or outdated"),
        "{}",
        notice.cause
    );
    let warns: Vec<&str> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warns.len(), 1, "one WARN for one discard: {logs}");
    Ok(())
}

#[test]
fn discard_surface_stale_root_reports_the_cause() -> TestResult {
    let (manager, project) = synced_project()?;
    {
        let connection = rusqlite::Connection::open(database(&project)).check("raw open")?;
        connection
            .execute(
                "UPDATE project SET root_path = '/elsewhere' WHERE id = 1",
                [],
            )
            .check("point the database at another root")?;
    }

    let (synced, _logs) = capture_warns(|| sync_project(&manager, &project));
    let synced = synced?;

    let notice = synced.discard.check("the rebuild is reported")?;
    assert!(
        notice.cause.contains("different project root"),
        "{}",
        notice.cause
    );
    Ok(())
}

#[test]
fn discard_surface_normal_sync_reports_nothing() -> TestResult {
    let (manager, project) = synced_project()?;

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));
    let synced = synced?;

    assert_eq!(synced.discard, None);
    assert!(
        !logs.contains("WARN"),
        "no warning on a normal sync: {logs}"
    );
    Ok(())
}

#[test]
fn discard_surface_is_reported_only_by_the_call_that_discarded() -> TestResult {
    let (manager, project) = synced_project()?;
    std::fs::write(database(&project), b"this is not a sqlite database").check("corrupt db")?;
    let first = sync_project(&manager, &project)?;
    let second = sync_project(&manager, &project)?;

    assert!(first.discard.is_some());
    assert_eq!(second.discard, None);
    Ok(())
}
