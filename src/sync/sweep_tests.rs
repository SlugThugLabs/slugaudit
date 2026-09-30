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
        &report.on_disk,
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

/// The single-walk contract: one Healthy sync call must walk the tree
/// exactly once, even when it also reconciles unreconciled events. The
/// defect this guards against walked a second time inside the event-driven
/// reconcile just to obtain `on_disk`. The file is marked dirty directly
/// (the default manager has no watcher) so the event branch actually runs —
/// without events the old path happened to walk once too. Counting is keyed
/// per root, so tests running in parallel cannot interfere.
#[test]
fn healthy_sync_walks_the_tree_exactly_once_with_unreconciled_events() {
    use crate::sync::SourceSyncManager;
    use crate::watch::WatcherHealth;

    let project = create_project();
    write(project.path(), "lib.rs", b"fn original() {}\n");
    let manager = SourceSyncManager::default();
    let _first = sync_project(&manager, &project);

    let state = manager
        .watch_state_for(project.path())
        .expect("project registered by the first sync");
    state.set_health(WatcherHealth::Healthy);

    let walks = super::count_walks(project.path());
    write(project.path(), "lib.rs", b"fn changed_and_longer() {}\n");
    state.mark_dirty("lib.rs".to_owned());
    let _second = sync_project(&manager, &project);

    assert_eq!(
        walks.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the Healthy path must walk the tree exactly once per call"
    );
}

/// The Healthy branch walks the tree once and feeds that single report to
/// both consumers: an importer added since the last revision, whose target
/// is on disk, must come back `Resolved` from the sweep-driven apply rather
/// than from a full publish. The default manager has no watcher, so forcing
/// health `Healthy` is what makes the sweep — not a full publish — the
/// healer.
#[test]
fn healthy_sweep_resolves_an_import_against_the_walked_on_disk_set() {
    use crate::sync::SourceSyncManager;
    use crate::watch::WatcherHealth;

    let project = create_project();
    write(project.path(), "lib.rs", b"fn original() {}\n");
    let manager = SourceSyncManager::default();
    let first = sync_project(&manager, &project);

    let state = manager
        .watch_state_for(project.path())
        .expect("project registered by the first sync");
    state.set_health(WatcherHealth::Healthy);

    write(project.path(), "pkg/__init__.py", b"");
    write(project.path(), "pkg/main.py", b"from .helper import f\n");
    write(project.path(), "pkg/helper.py", b"def f():\n    return 1\n");

    let second = sync_project(&manager, &project);
    assert_ne!(
        first.revision_id, second.revision_id,
        "the sweep must publish the files it discovered"
    );

    let connection = open_read_only(&second.database_path).expect("open served database");
    let kind: String = connection
        .query_row(
            "SELECT e.resolution_kind FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id WHERE f.path = 'pkg/main.py'",
            [],
            |row| row.get(0),
        )
        .expect("pkg/main.py's edge");
    assert_eq!(
        kind, "Resolved",
        "the Healthy sweep path must resolve the import against the walked on-disk set"
    );
}

/// The Healthy path reuses the one sweep report for both consumers, so the
/// apply resolves imports against the same `on_disk` the walk produced. A
/// target too large for the reconcile's per-file ceiling is the only way a
/// discovered file can stay unindexed — nothing but `on_disk` can resolve
/// its importer — so the edge must still be `Resolved`, never the
/// `Unresolved` an agent would read as dead code.
///
/// `ensure_current` draws its limits from a process-wide `OnceLock`, and
/// only the full-publish path consults the project's configured overrides,
/// so the ceiling cannot be injected through the manager. This drives the
/// exact two steps the Healthy branch drives — one walk, then the apply —
/// with explicit limits, on a watcher-less manager whose health is forced
/// `Healthy`.
#[test]
fn healthy_sweep_apply_resolves_an_import_to_a_discovered_but_skipped_file() {
    use crate::model::ResourceLimits;
    use crate::store::open_read_write;
    use crate::sync::SourceSyncManager;
    use crate::watch::WatcherHealth;

    let project = create_project();
    write(project.path(), "lib.rs", b"fn original() {}\n");
    let manager = SourceSyncManager::default();
    let first = sync_project(&manager, &project);
    let state = manager
        .watch_state_for(project.path())
        .expect("project registered by the first sync");
    state.set_health(WatcherHealth::Healthy);

    write(project.path(), "pkg/__init__.py", b"");
    write(project.path(), "pkg/main.py", b"from .helper import f\n");
    // Over `max_file_bytes` below: the walk discovers it, sampling never
    // indexes it. It has no `files` row, so only `on_disk` can say it exists.
    write(project.path(), "pkg/helper.py", &[b'#'; 200]);

    let mut connection = open_read_write(&first.database_path).expect("open read-write");
    let limits = ResourceLimits {
        max_file_bytes: 50,
        ..ResourceLimits::default()
    };
    let options = ReconcileOptions {
        limits,
        deadline: Deadline::after(limits.max_sync_wall_clock),
        rules: None,
    };

    // Step 1: the single walk the Healthy branch performs.
    let report = sweep(&connection, project.path(), &options.deadline).expect("sweep");
    // Step 2: apply that report's findings with its own `on_disk`.
    let (candidates, _deleted, _unchanged) = crate::sync::sweep_reconcile::apply_sweep_report(
        project.path(),
        &mut connection,
        report,
        &options,
    )
    .expect("apply the sweep report");

    assert_eq!(candidates, 3, "all three new files are candidates");
    let indexed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path = 'pkg/helper.py'",
            [],
            |row| row.get(0),
        )
        .expect("count indexed helper");
    assert_eq!(indexed, 0, "the oversize target must stay unindexed");
    let kind: String = connection
        .query_row(
            "SELECT e.resolution_kind FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id WHERE f.path = 'pkg/main.py'",
            [],
            |row| row.get(0),
        )
        .expect("pkg/main.py's edge");
    assert_eq!(
        kind, "Resolved",
        "the apply must resolve imports against the sweep's on_disk, not indexed rows alone"
    );
}
