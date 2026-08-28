//! Sweep classification tests plus the watcher-independence regression
//! test: a change no watcher event ever reported must still be reconciled
//! by `ensure_current` before the tool call serves evidence.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::sweep;
use crate::model::process_limits;
use crate::store::open_read_only;
use crate::sync::reconcile::{ReconcileOptions, reconcile_dirty_paths_with_deadline};
use crate::sync::test_support::{create_project, setup_project, stored_paths, sync_project, write};
use crate::util::{Deadline, mtime_unix_seconds};

/// The stat fingerprint compares whole seconds; a second boundary crossing
/// between two back-to-back stats would flake a classification test. This
/// parks the test at the start of a fresh second so every stat that follows
/// lands in the same one.
fn sleep_into_a_fresh_second() {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .subsec_millis();
    if millis > 800 {
        std::thread::sleep(Duration::from_millis(1000 - u64::from(millis) + 10));
    }
}

/// Overwrites each file's stored stat fingerprint with values from a stat
/// performed right now, so classification is deterministic regardless of
/// what the publish recorded.
fn store_current_stats(connection: &rusqlite::Connection, root: &Path, paths: &[&str]) {
    sleep_into_a_fresh_second();
    for path in paths {
        let metadata = std::fs::metadata(root.join(path)).expect("stat fixture file");
        connection
            .execute(
                "UPDATE files SET modified_unix_seconds = ?1, byte_len = ?2 WHERE path = ?3",
                rusqlite::params![
                    mtime_unix_seconds(&metadata),
                    i64::try_from(metadata.len()).expect("file length fits i64"),
                    path
                ],
            )
            .expect("update stored stat fingerprint");
    }
}

fn sweep_report(connection: &rusqlite::Connection, root: &Path) -> super::SweepReport {
    let deadline = Deadline::after(process_limits().max_sync_wall_clock);
    sweep(connection, root, &deadline).expect("sweep succeeds")
}

fn reconcile_sweep(connection: &mut rusqlite::Connection, root: &Path, report: super::SweepReport) {
    let limits = *process_limits();
    let options = ReconcileOptions {
        limits,
        deadline: Deadline::after(limits.max_sync_wall_clock),
        rules: None,
    };
    let expected = crate::sync::manager_meta::current_revision_id(connection)
        .expect("read the current revision");
    reconcile_dirty_paths_with_deadline(
        connection,
        root,
        report.candidates,
        report.deleted,
        expected.as_deref(),
        &options,
    )
    .expect("reconcile succeeds");
}

#[test]
fn an_unchanged_project_nominates_nothing() {
    let (project, _db_dir, connection, _revision) = setup_project();
    store_current_stats(&connection, project.path(), &["a.rs", "b.rs"]);

    let report = sweep_report(&connection, project.path());
    assert!(
        report.candidates.is_empty(),
        "unexpected candidates: {:?}",
        report.candidates
    );
    assert!(report.deleted.is_empty());
    assert_eq!(report.unchanged, 2);
}

#[test]
fn a_new_file_is_nominated_and_reconciles_in() {
    let (project, _db_dir, mut connection, _revision) = setup_project();
    store_current_stats(&connection, project.path(), &["a.rs", "b.rs"]);
    write(project.path(), "c.rs", b"fn c() {}\n");

    let report = sweep_report(&connection, project.path());
    assert!(
        report.candidates.contains("c.rs"),
        "{:?}",
        report.candidates
    );
    reconcile_sweep(&mut connection, project.path(), report);

    assert_eq!(stored_paths(&connection), ["a.rs", "b.rs", "c.rs"]);
}

#[test]
fn a_changed_file_reconciles_to_new_content() {
    let (project, _db_dir, mut connection, _revision) = setup_project();
    // Different length: the candidate is deterministic even when the edit
    // lands in the same wall-clock second as the stored fingerprint.
    write(project.path(), "a.rs", b"fn a_much_longer_body() {}\n");

    let report = sweep_report(&connection, project.path());
    assert!(
        report.candidates.contains("a.rs"),
        "{:?}",
        report.candidates
    );
    reconcile_sweep(&mut connection, project.path(), report);

    let content: String = connection
        .query_row("SELECT content FROM files WHERE path = 'a.rs'", [], |row| {
            row.get(0)
        })
        .expect("row exists");
    assert!(
        content.contains("a_much_longer_body"),
        "stale content served: {content}"
    );
}

#[test]
fn a_deleted_file_converges_out() {
    let (project, _db_dir, mut connection, _revision) = setup_project();
    store_current_stats(&connection, project.path(), &["a.rs"]);
    std::fs::remove_file(project.path().join("b.rs")).expect("remove fixture");

    let report = sweep_report(&connection, project.path());
    assert!(report.deleted.contains("b.rs"), "{:?}", report.deleted);
    assert!(
        report.candidates.is_empty(),
        "a.rs must still match: {:?}",
        report.candidates
    );
    reconcile_sweep(&mut connection, project.path(), report);

    assert_eq!(stored_paths(&connection), ["a.rs"]);
}

#[test]
fn a_file_that_became_gitignored_converges_out() {
    let (project, _db_dir, mut connection, _revision) = setup_project();
    store_current_stats(&connection, project.path(), &["b.rs"]);
    // a.rs is already indexed; ignoring it makes the walker prune it, so it
    // must land in the deleted set exactly as a full publish would converge.
    write(project.path(), ".gitignore", b"a.rs\n");

    let report = sweep_report(&connection, project.path());
    assert!(
        report.deleted.contains("a.rs"),
        "deleted={:?} candidates={:?}",
        report.deleted,
        report.candidates
    );
    reconcile_sweep(&mut connection, project.path(), report);

    let paths = stored_paths(&connection);
    assert!(
        !paths.iter().any(|path| path == "a.rs"),
        "gitignored file must converge out: {paths:?}"
    );
    assert!(
        paths.iter().any(|path| path == ".gitignore"),
        "the ignore file itself is indexed: {paths:?}"
    );
}

#[test]
fn an_equal_hash_candidate_refreshes_its_stat_and_publishes_nothing() {
    let (project, _db_dir, mut connection, _revision) = setup_project();
    store_current_stats(&connection, project.path(), &["a.rs", "b.rs"]);
    // Simulate stat-only drift (touch / no-op save): a.rs's stored mtime is
    // an hour in the future, so it is nominated on every sweep until the
    // hash-equal refresh stops it.
    connection
        .execute(
            "UPDATE files SET modified_unix_seconds = modified_unix_seconds + 3600 WHERE path = 'a.rs'",
            [],
        )
        .expect("bump stored mtime");
    let revisions_before: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("count revisions");

    let report = sweep_report(&connection, project.path());
    assert!(
        report.candidates.contains("a.rs"),
        "{:?}",
        report.candidates
    );

    // The refresh stat inside the reconcile and the second sweep's stat
    // must land in the same wall-clock second for the final assertion.
    sleep_into_a_fresh_second();
    reconcile_sweep(&mut connection, project.path(), report);

    let revisions_after: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("count revisions");
    assert_eq!(
        revisions_after, revisions_before,
        "unchanged content must not publish a revision"
    );

    let second = sweep_report(&connection, project.path());
    assert!(
        second.candidates.is_empty(),
        "the refreshed stat fingerprint must stop re-nomination: {:?}",
        second.candidates
    );
}

/// The regression test for watcher independence: with no watcher events at
/// all (the default manager has no watcher; its state is forced Healthy so
/// the sweep — not a full publish — is what must heal changes), a file edit
/// is still reconciled before the tool call serves evidence.
#[test]
fn ensure_current_heals_a_change_the_watcher_never_reported() {
    use crate::sync::SourceSyncManager;
    use crate::watch::WatcherHealth;

    let project = create_project();
    write(project.path(), "lib.rs", b"fn original_name() {}\n");
    let manager = SourceSyncManager::default();

    let first = sync_project(&manager, &project);

    // Force the Healthy path: the sweep, not a full publish, is what must
    // find the change.
    let state = manager
        .watch_state_for(project.path())
        .expect("project registered by the first sync");
    state.set_health(WatcherHealth::Healthy);

    // Different length: the candidate is deterministic across second
    // boundaries.
    write(project.path(), "lib.rs", b"fn changed_and_longer() {}\n");
    let second = sync_project(&manager, &project);

    assert_ne!(
        first.revision_id, second.revision_id,
        "a change no watcher reported must still publish a revision"
    );
    let connection = open_read_only(&second.database_path).expect("open served database");
    let content: String = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("row exists");
    assert!(
        content.contains("changed_and_longer"),
        "stale content served after sync: {content}"
    );
}
