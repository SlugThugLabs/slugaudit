use super::*;
use crate::store::test_capture::capture_warns;
use crate::sync::SourceSyncManager;
use crate::sync::test_support::{create_project, sync_project, write_file};

fn database(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning/slugaudit/project.db")
}

fn seed_findings(database: &Path, count: usize) {
    let connection = crate::store::open_read_write(database).expect("open db");
    for index in 0..count {
        connection
            .execute(
                "INSERT INTO findings (path, source_hash, line_start, line_end, \
                 severity, category, title, description, created_at_unix, \
                 evidence_revision, status, session_id) \
                 VALUES (?1, 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', '')",
                rusqlite::params![format!("f{index}.rs")],
            )
            .expect("seed finding");
    }
}

fn synced_project_with_findings(count: usize) -> (SourceSyncManager, tempfile::TempDir) {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);
    seed_findings(&database(&project), count);
    (manager, project)
}

#[test]
fn discard_surface_schema_mismatch_reports_the_finding_count_and_logs_once() {
    let (manager, project) = synced_project_with_findings(3);
    {
        let connection = rusqlite::Connection::open(database(&project)).expect("raw open");
        connection
            .execute_batch("PRAGMA user_version = 999;")
            .expect("foreign schema version");
    }

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));

    let notice = synced.discard.expect("the rebuild is reported");
    assert_eq!(notice.findings_discarded, Some(3));
    assert!(
        notice.cause.contains("corrupt or outdated"),
        "cause was {}",
        notice.cause
    );
    let warns: Vec<&str> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warns.len(), 1, "one WARN for one discard: {logs}");
    assert!(warns[0].contains("findings_discarded=Some(3)"), "{logs}");
    assert!(warns[0].contains("error="), "the cause is logged: {logs}");
}

#[test]
fn discard_surface_corrupt_database_reports_unknown_not_zero() {
    let (manager, project) = synced_project_with_findings(2);
    std::fs::write(database(&project), b"this is not a sqlite database").expect("corrupt db");

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));

    let notice = synced.discard.expect("the rebuild is reported");
    assert_eq!(notice.findings_discarded, None);
    let warns: Vec<&str> = logs.lines().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warns.len(), 1, "one WARN for one discard: {logs}");
    assert!(warns[0].contains("findings_discarded=None"), "{logs}");
    assert!(warns[0].contains("count_error="), "{logs}");
}

#[test]
fn discard_surface_stale_root_reports_the_finding_count() {
    let (manager, project) = synced_project_with_findings(4);
    {
        let connection = rusqlite::Connection::open(database(&project)).expect("raw open");
        connection
            .execute(
                "UPDATE project SET root_path = '/elsewhere' WHERE id = 1",
                [],
            )
            .expect("point the database at another root");
    }

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));

    let notice = synced.discard.expect("the rebuild is reported");
    assert_eq!(notice.findings_discarded, Some(4));
    assert!(logs.contains("findings_discarded=Some(4)"), "{logs}");
}

#[test]
fn discard_surface_normal_sync_reports_nothing() {
    let (manager, project) = synced_project_with_findings(2);

    let (synced, logs) = capture_warns(|| sync_project(&manager, &project));

    assert_eq!(synced.discard, None);
    assert!(
        !logs.contains("WARN"),
        "no warning on a normal sync: {logs}"
    );
}

#[test]
fn discard_surface_is_reported_only_by_the_call_that_discarded() {
    let (manager, project) = synced_project_with_findings(1);
    std::fs::write(database(&project), b"this is not a sqlite database").expect("corrupt db");
    let first = sync_project(&manager, &project);
    let second = sync_project(&manager, &project);

    assert!(first.discard.is_some());
    assert_eq!(second.discard, None);
}
