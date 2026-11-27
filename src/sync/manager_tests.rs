//! Integration tests for `SourceSyncManager`'s full-scan synchronization,
//! lock lifecycle, progress framing, and real-path filesystem transitions.

use super::*;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::sync::test_support::{create_project, sync_project, write_file};
use crate::tools::context::with_verified_read;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::fs;

fn db_error(error: &rusqlite::Error) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

fn stored_file_count(synced: &SyncedProject) -> TestResult<i64> {
    with_verified_read(synced, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(|error| db_error(&error))
    })
    .check("read file count")
}

fn stored_file_content(synced: &SyncedProject, path: &str) -> TestResult<Option<String>> {
    with_verified_read(synced, |tx| {
        tx.query_row("SELECT content FROM files WHERE path = ?1", [path], |row| {
            row.get(0)
        })
        .map_err(|error| db_error(&error))
    })
    .check("read file content")
}

fn read_stored_paths(synced: &SyncedProject) -> TestResult<Vec<String>> {
    with_verified_read(synced, |tx| {
        crate::sync::test_support::stored_paths(tx)
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))
    })
    .check("read stored paths")
}

#[test]
fn edit_file_between_calls_returns_fresh_evidence() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn value() -> i32 { 42 }\n")?;

    let synced = sync_project(&manager, &project)?;
    let value_a_revision = synced.revision_id.clone();

    let content_a = stored_file_content(&synced, "lib.rs")?;
    ensure(
        content_a.check("read file back")?.contains("42"),
        "initial content contains 42",
    )?;

    write_file(&project, "lib.rs", b"pub fn value() -> i32 { 99 }\n")?;

    let synced_b = sync_project(&manager, &project)?;
    ensure_ne(
        &value_a_revision,
        &synced_b.revision_id,
        "edited file publishes a new revision",
    )?;

    let content_b = stored_file_content(&synced_b, "lib.rs")?;
    ensure(
        content_b.check("read file back")?.contains("99"),
        "updated content contains 99",
    )?;
    Ok(())
}

#[test]
fn global_file_size_limit_governs_the_scan() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    let _config = crate::tools::test_support::use_global_config(r#"{"max_file_bytes":50}"#);
    write_file(&project, "lib.rs", b"pub fn initially_small() {}\n")?;
    let first = sync_project(&manager, &project)?;

    write_file(&project, "lib.rs", &[b'x'; 200])?;
    let second = sync_project(&manager, &project)?;

    ensure_ne(
        &first.revision_id,
        &second.revision_id,
        "first.revision_id != second.revision_id",
    )?;
    let (kind, content): (String, Option<String>) = with_verified_read(&second, |tx| {
        tx.query_row(
            "SELECT file_kind, content FROM files WHERE path = 'lib.rs'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| db_error(&error))
    })
    .check("read scanned file state")?;
    ensure_eq(&kind, &"skipped", "the project limit must govern the scan")?;
    ensure_eq(&content, &None, "an oversized file must retain no content")?;
    Ok(())
}

#[test]
fn create_new_file_gets_indexed() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn a() {}\n")?;

    let synced = sync_project(&manager, &project)?;
    ensure_eq(&stored_file_count(&synced)?, &1, "file_count == 1")?;

    write_file(&project, "new.rs", b"pub fn b() {}\n")?;

    let synced2 = sync_project(&manager, &project)?;
    ensure_eq(&stored_file_count(&synced2)?, &2, "file_count2 == 2")?;
    Ok(())
}

#[test]
fn delete_file_gets_removed_from_index() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn a() {}\n")?;
    write_file(&project, "to_delete.rs", b"pub fn b() {}\n")?;

    let synced = sync_project(&manager, &project)?;
    ensure_eq(&stored_file_count(&synced)?, &2, "file_count == 2")?;

    fs::remove_file(project.path().join("to_delete.rs")).check("delete file")?;

    let synced2 = sync_project(&manager, &project)?;
    ensure_eq(&stored_file_count(&synced2)?, &1, "file_count2 == 1")?;
    Ok(())
}

#[test]
fn write_same_contents_does_not_create_new_revision() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    let content = b"pub fn a() {}\n";
    write_file(&project, "lib.rs", content)?;

    let synced = sync_project(&manager, &project)?;
    let revision = synced.revision_id;

    write_file(&project, "lib.rs", content)?;

    let synced2 = sync_project(&manager, &project)?;
    ensure_eq(
        &revision,
        &synced2.revision_id,
        "revision == synced2.revision_id",
    )?;
    Ok(())
}

#[test]
fn multiple_rapid_edits_collapse_to_one_revision() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn a() {}\n")?;

    let synced = sync_project(&manager, &project)?;
    let old_revision = synced.revision_id;

    for i in 0..5 {
        write_file(
            &project,
            "lib.rs",
            format!("pub fn a{i}() {{}}\n").as_bytes(),
        )?;
    }

    let synced2 = sync_project(&manager, &project)?;
    ensure_ne(
        &old_revision,
        &synced2.revision_id,
        "old_revision != synced2.revision_id",
    )?;
    Ok(())
}

#[test]
fn restart_after_offline_changes_detects_changes() -> TestResult {
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn original() {}\n")?;

    let manager1 = SourceSyncManager::new();
    let synced1 = sync_project(&manager1, &project)?;
    let revision1 = synced1.revision_id;
    drop(manager1);

    write_file(&project, "lib.rs", b"pub fn changed() {}\n")?;

    let manager2 = SourceSyncManager::new();
    let synced2 = sync_project(&manager2, &project)?;
    ensure_ne(
        &revision1,
        &synced2.revision_id,
        "revision1 != synced2.revision_id",
    )?;
    Ok(())
}

#[test]
fn delete_then_recreate_gets_reindexed() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "lib.rs", b"pub fn original() {}\n")?;

    let _synced = sync_project(&manager, &project)?;

    fs::remove_file(project.path().join("lib.rs")).check("delete file")?;
    write_file(&project, "lib.rs", b"pub fn recreated() {}\n")?;

    let synced2 = sync_project(&manager, &project)?;
    let content = stored_file_content(&synced2, "lib.rs")?;
    ensure(
        content
            .check("read file back")?
            .contains("pub fn recreated()"),
        "recreated content is indexed",
    )?;
    Ok(())
}

#[test]
fn ensure_current_emits_completed_on_early_error_path() -> TestResult {
    struct RecordingSink {
        events: Mutex<Vec<(&'static str, &'static str)>>,
    }
    impl ProgressSink for RecordingSink {
        fn emit(&self, event: ProgressEvent) {
            let entry = match event {
                ProgressEvent::Started { phase } => ("started", phase),
                ProgressEvent::Completed { phase } => ("completed", phase),
                ProgressEvent::Sampling { phase, .. } => ("sampling", phase),
            };
            if let Ok(mut guard) = self.events.lock() {
                guard.push(entry);
            }
        }
    }

    let unactivated = tempfile::tempdir().check("unactivated dir")?;
    let manager = SourceSyncManager::new();
    let sink = RecordingSink {
        events: Mutex::new(Vec::new()),
    };

    let _error = manager
        .ensure_current(&unactivated.path().to_string_lossy(), &sink)
        .map(|_| ())
        .check_err("unactivated project must fail")?;

    let events = sink.events.lock().check("lock recorded events")?;
    let expected: &[(&str, &str)] = &[
        ("started", "ensuring_current"),
        ("completed", "ensuring_current"),
    ];
    ensure_eq(
        &events.as_slice(),
        &expected,
        "early error path must emit Completed after Started",
    )?;
    Ok(())
}

#[test]
fn project_sync_lock_shares_live_mutex_and_prunes_dropped_entries() -> TestResult {
    let manager = SourceSyncManager::default();
    let root_a = PathBuf::from("/tmp/slugaudit-lock-a");
    let root_b = PathBuf::from("/tmp/slugaudit-lock-b");

    let lock_a1 = manager.project_sync_lock(&root_a).check("lock a1")?;
    let lock_a2 = manager.project_sync_lock(&root_a).check("lock a2")?;
    ensure(
        Arc::ptr_eq(&lock_a1, &lock_a2),
        "concurrent callers on the same root share one mutex",
    )?;
    drop(lock_a1);
    drop(lock_a2);

    let _lock_b = manager.project_sync_lock(&root_b).check("lock b")?;
    let table = crate::util::lock(&manager.sync_locks, "sync_locks").check("inspect table")?;
    ensure_eq(
        &table.len(),
        &1,
        "expired weak entry for root_a was pruned on next lookup",
    )?;
    Ok(())
}

#[test]
fn mv_rename_between_syncs_updates_files_and_reresolves_mod_and_use_edges() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(
        &project,
        "src/lib.rs",
        b"mod new;\nuse crate::new::greet;\npub fn call() { greet(); }\n",
    )?;
    write_file(&project, "src/old.rs", b"pub fn greet() {}\n")?;

    let first = sync_project(&manager, &project)?;
    ensure_eq(
        &read_stored_paths(&first)?,
        &vec!["src/lib.rs".to_owned(), "src/old.rs".to_owned()],
        "initial sync indexes src/lib.rs and src/old.rs",
    )?;

    fs::rename(
        project.path().join("src/old.rs"),
        project.path().join("src/new.rs"),
    )
    .check("mv src/old.rs src/new.rs")?;

    let second = sync_project(&manager, &project)?;
    ensure_ne(
        &first.revision_id,
        &second.revision_id,
        "rename publishes a new revision",
    )?;
    let after_paths = read_stored_paths(&second)?;
    let resolved_targets: Vec<String> = with_verified_read(&second, |tx| {
        let mut stmt = tx
            .prepare(
                "SELECT t.path FROM dependency_edges e \
                 JOIN files f ON f.id = e.from_file_id \
                 JOIN files t ON t.id = e.to_file_id \
                 WHERE f.path = 'src/lib.rs' AND e.resolution_kind = 'Resolved' \
                 ORDER BY e.raw_import_text",
            )
            .map_err(|e| db_error(&e))?;
        let targets = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| db_error(&e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| db_error(&e))?;
        Ok(targets)
    })
    .check("read renamed edges")?;
    ensure_eq(
        &after_paths,
        &vec!["src/lib.rs".to_owned(), "src/new.rs".to_owned()],
        "mv removes src/old.rs and adds src/new.rs",
    )?;
    ensure_eq(
        &resolved_targets,
        &vec!["src/new.rs".to_owned(), "src/new.rs".to_owned()],
        "both mod new; and use crate::new::greet; re-resolve to src/new.rs",
    )?;
    Ok(())
}

#[test]
fn regular_file_replaced_by_directory_between_syncs_reindexes_cleanly() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "src/lib.rs", b"mod foo;\n")?;
    write_file(&project, "src/foo", b"plain regular file\n")?;

    let first = sync_project(&manager, &project)?;
    ensure_eq(
        &read_stored_paths(&first)?,
        &vec!["src/foo".to_owned(), "src/lib.rs".to_owned()],
        "src/foo regular file is indexed initially",
    )?;

    fs::remove_file(project.path().join("src/foo")).check("remove regular file src/foo")?;
    write_file(
        &project,
        "src/foo/mod.rs",
        b"pub fn inside_dir() -> usize { 1 }\n",
    )?;

    let second = sync_project(&manager, &project)?;
    ensure_ne(
        &first.revision_id,
        &second.revision_id,
        "file-to-directory replacement publishes a new revision",
    )?;
    let after_paths = read_stored_paths(&second)?;
    let mod_target: Option<String> = with_verified_read(&second, |tx| {
        tx.query_row(
            "SELECT t.path FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             LEFT JOIN files t ON t.id = e.to_file_id \
             WHERE f.path = 'src/lib.rs'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|e| db_error(&e))
    })
    .check("read replaced directory edge")?;
    ensure_eq(
        &after_paths,
        &vec!["src/foo/mod.rs".to_owned(), "src/lib.rs".to_owned()],
        "src/foo file row is removed and src/foo/mod.rs is indexed",
    )?;
    ensure_eq(
        &mod_target.as_deref(),
        &Some("src/foo/mod.rs"),
        "mod foo; resolves to src/foo/mod.rs",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn unix_fifo_inside_active_project_tree_is_ignored_without_hanging_sync() -> TestResult {
    let manager = SourceSyncManager::new();
    let project = create_project()?;
    write_file(&project, "src/lib.rs", b"pub fn live() {}\n")?;
    let first = sync_project(&manager, &project)?;

    let fifo_path = project.path().join("src/unopened_writer.fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .check("spawn mkfifo")?;
    ensure(status.success(), "mkfifo created a named pipe")?;

    write_file(&project, "src/extra.rs", b"pub fn extra() {}\n")?;
    let second = sync_project(&manager, &project)?;
    ensure_ne(
        &first.revision_id,
        &second.revision_id,
        "sync completes past the FIFO without hanging",
    )?;
    ensure_eq(
        &read_stored_paths(&second)?,
        &vec!["src/extra.rs".to_owned(), "src/lib.rs".to_owned()],
        "FIFO is excluded cleanly while regular files sync normally",
    )?;
    Ok(())
}
