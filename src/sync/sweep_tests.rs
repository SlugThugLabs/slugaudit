//! Sweep classification tests plus the watcher-independence regression
//! test: a change no watcher event ever reported must still be reconciled
//! by `ensure_current` before the tool call serves evidence.

use std::collections::HashSet;
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
    // The oversize target must stay unindexed — but it must not vanish
    // either: since the existence-row change it is a `skipped` row with no
    // content, so `count(*)` still sees it while nothing pretends it was
    // read. The old assertion (`COUNT(*) == 0`) encoded the defect this
    // phase removes: a discovered-but-skipped file was invisible in every
    // count the tools return.
    let (kind, content, completeness, outcome): (String, Option<String>, String, String) =
        connection
            .query_row(
                "SELECT file_kind, content, extraction_completeness, parse_outcome \
                 FROM files WHERE path = 'pkg/helper.py'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("the oversize target must have an existence row");
    assert_eq!(kind, "skipped", "the oversize target must stay unindexed");
    assert_eq!(content, None, "no content is retained for a skipped file");
    assert_eq!(completeness, "Unavailable");
    assert_eq!(outcome, "NotAttempted");
    let edge_kind: String = connection
        .query_row(
            "SELECT e.resolution_kind FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id WHERE f.path = 'pkg/main.py'",
            [],
            |row| row.get(0),
        )
        .expect("pkg/main.py's edge");
    assert_eq!(
        edge_kind, "Resolved",
        "the apply must resolve imports against the sweep's on_disk, not indexed rows alone"
    );
}

#[test]
fn a_stored_unknown_size_is_always_nominated_for_re_examination() {
    let (project, _db_dir, connection, _revision) = setup_project();
    store_current_stats(&connection, project.path(), &["a.rs", "b.rs"]);
    connection
        .execute("UPDATE files SET byte_len = NULL WHERE path = 'a.rs'", [])
        .expect("store an unknown size");

    let report = sweep_report(&connection, project.path());

    assert!(
        report.candidates.contains("a.rs"),
        "NULL byte_len must never match a real size: {:?}",
        report.candidates
    );
    assert!(!report.candidates.contains("b.rs"));
}

/// The first regular-file entry the indexable walker yields under `root`.
fn first_walked_file(root: &Path) -> ignore::DirEntry {
    crate::ignore_rules::indexable_walker(root)
        .filter_map(Result::ok)
        .find(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .expect("the walker yields the fixture file")
}

#[test]
fn a_stat_failing_with_something_other_than_not_found_leaves_the_file_unverifiable_not_gone() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "d/a.rs", b"fn a() {}");
    let entry = first_walked_file(project.path());
    // After the walk, `d` becomes a regular file: stat of `d/a.rs` now fails
    // with ENOTDIR, which is not `NotFound`.
    std::fs::remove_dir_all(project.path().join("d")).expect("remove the directory");
    write(project.path(), "d", b"now a file");

    let walked = super::walked_file(project.path(), Ok(entry), &mut HashSet::new());

    assert!(
        matches!(&walked, super::Walked::Unverifiable(path) if path == "d/a.rs"),
        "a non-NotFound stat failure must not classify the file as gone"
    );
}

#[test]
fn a_walked_file_that_vanished_before_its_stat_is_skipped_so_its_row_is_deleted() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "a.rs", b"fn a() {}");
    let entry = first_walked_file(project.path());
    std::fs::remove_file(project.path().join("a.rs")).expect("delete the file");

    let walked = super::walked_file(project.path(), Ok(entry), &mut HashSet::new());

    assert!(
        matches!(walked, super::Walked::Skipped),
        "NotFound is the one stat error that means deleted"
    );
}

/// The sweep-driven apply's failure arm: the walk succeeded, but applying its
/// findings exhausted the shared budget. The failure must be logged under the
/// apply operation (never as a failed walk), carry the typed budget code, and
/// mark the watcher `Desynced`.
#[test]
fn a_failed_sweep_apply_names_the_apply_operation_and_marks_the_watcher_desynced() {
    use crate::progress::NoopProgressSink;
    use crate::watch::{WatchState, WatcherHealth};

    let (project, _db_dir, mut connection, _revision) = setup_project();
    let limits = *process_limits();
    let options = ReconcileOptions {
        limits,
        deadline: Deadline::after(Duration::ZERO),
        rules: None,
    };
    let report = super::SweepReport {
        candidates: ["a.rs".to_owned()].into_iter().collect(),
        deleted: HashSet::new(),
        on_disk: ["a.rs".to_owned()].into_iter().collect(),
        unchanged: 0,
        unrepresentable: HashSet::new(),
    };
    let state = WatchState::new();
    let sink = NoopProgressSink;

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-sweep-apply-failure-probe");
        crate::sync::sweep_reconcile::sweep_and_reconcile(
            project.path(),
            report,
            &options,
            &mut connection,
            &state,
            &sink,
        )
    });

    let error = result.expect_err("a spent apply budget must fail the sweep");
    assert!(
        error.message.contains("reconciling sweep results"),
        "the returned error must come from the apply: {error:?}"
    );
    assert_eq!(
        error.data.as_ref().and_then(|data| data["code"].as_str()),
        Some("sync_budget_exhausted"),
        "the budget failure must keep its machine-readable code: {error:?}"
    );
    assert_eq!(
        state.health(),
        WatcherHealth::Desynced,
        "a failed apply leaves freshness ambiguous"
    );
    assert!(
        logs.contains("control-line-sweep-apply-failure-probe"),
        "the capture must be live: {logs}"
    );
    let apply_lines: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("operation=\"sweep_reconcile_apply\""))
        .collect();
    assert_eq!(
        apply_lines.len(),
        1,
        "the apply failure must be logged exactly once: {logs}"
    );
    assert!(
        apply_lines[0].contains("sweep reconcile apply failed"),
        "the log must name the failed operation: {}",
        apply_lines[0]
    );
    assert!(
        apply_lines[0].contains("error_code=\"sync_budget_exhausted\""),
        "the log must carry the typed failure's machine code: {}",
        apply_lines[0]
    );
    assert!(
        !apply_lines[0].contains("a.rs"),
        "the log must not carry a project-relative path: {}",
        apply_lines[0]
    );
    assert!(
        !logs.contains("stat sweep walk failed"),
        "a walk that succeeded must not be reported as failed: {logs}"
    );
}

/// The incremental path never re-runs full discovery, so the stat sweep is
/// the only walk that can see a non-UTF-8 file added or removed between
/// full publishes. `ensure_current` must persist the sweep's set, or
/// `report` would keep a stale count until the next full publish.
#[cfg(unix)]
#[test]
fn the_healthy_sync_path_refreshes_the_persisted_non_utf8_set() {
    use crate::sync::SourceSyncManager;
    use std::os::unix::ffi::OsStringExt;

    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write(project.path(), "a.rs", b"fn a() {}\n");
    sync_project(&manager, &project);
    let database = project.path().join(".planning/slugaudit/project.db");
    let stored_count = || {
        let connection = open_read_only(&database).expect("open the published database");
        connection
            .query_row("SELECT count(*) FROM unrepresentable_paths", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count the persisted set")
    };
    assert_eq!(stored_count(), 0);

    let bad = project
        .path()
        .join(std::ffi::OsString::from_vec(b"bad\xff.rs".to_vec()));
    std::fs::write(&bad, b"fn b() {}\n").expect("write non-utf8 file");
    std::thread::sleep(Duration::from_millis(200));
    sync_project(&manager, &project);
    assert_eq!(stored_count(), 1, "the sweep sees the added file");

    std::fs::remove_file(&bad).expect("remove non-utf8 file");
    std::thread::sleep(Duration::from_millis(200));
    sync_project(&manager, &project);
    assert_eq!(stored_count(), 0, "the sweep sees the removal");
}
