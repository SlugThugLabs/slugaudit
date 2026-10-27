//! Proofs that the result store's files (`<runtime>/results/*.db` and the
//! owner lock) are invisible to every path that could index or react to
//! them: discovery and full publish, the stat sweep, the watcher scope and
//! reconcile. Each test plants real store files (a real `ResultStore` opened
//! through the manager's slot, plus a stray `.db`) and drives the real code.

use super::ResultStoreSlot;
use crate::model::process_limits;
use crate::progress::NoopProgressSink;
use crate::store::open_read_only;
use crate::sync::SourceSyncManager;
use crate::sync::reconcile::{ReconcileOptions, reconcile_dirty_paths_with_deadline};
use crate::sync::sweep::sweep;
use crate::sync::test_support::{create_project, setup_project, stored_paths, sync_project, write};
use crate::util::Deadline;
use crate::watch::{WatchManager, WatchScope, WatcherHealth};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RESULTS: &str = ".planning/slugaudit/results";

/// Opens a real store under `<root>/.planning/slugaudit` (which creates the
/// results directory and the owner lock) and plants a stray result file.
/// Returns the slot so the store (and its lock) stays alive, and the
/// project-relative paths of everything now under the results directory.
fn plant_store_files(root: &Path) -> (ResultStoreSlot, Vec<String>) {
    let slot = ResultStoreSlot::default();
    let runtime_dir = root.join(".planning/slugaudit");
    std::fs::create_dir_all(&runtime_dir).expect("create the runtime directory");
    slot.for_dir(&runtime_dir)
        .expect("open the store under the project");
    write(root, &format!("{RESULTS}/stray-result.db"), b"not a result");
    let mut planted: Vec<String> = std::fs::read_dir(root.join(RESULTS))
        .expect("list the results directory")
        .map(|entry| {
            format!(
                "{RESULTS}/{}",
                entry.expect("results entry").file_name().to_string_lossy()
            )
        })
        .collect();
    planted.sort();
    assert!(
        planted.iter().any(|path| path.ends_with(".lock")),
        "the owner lock exists: {planted:?}"
    );
    assert!(
        planted.iter().any(|path| path.ends_with("stray-result.db")),
        "the stray result file exists: {planted:?}"
    );
    (slot, planted)
}

fn mentions_results(path: &str) -> bool {
    path.contains("results") || path.contains(".lock") || path.ends_with(".db")
}

#[test]
fn result_store_proof_results_dir_and_lock_file_are_not_discovered() {
    let project = create_project();
    write(project.path(), "lib.rs", b"pub fn a() {}\n");
    let (_slot, planted) = plant_store_files(project.path());
    let manager = SourceSyncManager::with_watcher();

    let synced = sync_project(&manager, &project);

    let connection = open_read_only(&synced.database_path).expect("open the index");
    let paths = stored_paths(&connection);
    assert_eq!(paths, ["lib.rs"], "planted files: {planted:?}");
    let (discovered, _skipped) =
        crate::sync::discovery::discover(project.path()).expect("discovery succeeds");
    let discovered: Vec<String> = discovered
        .into_iter()
        .map(|file| file.relative_path)
        .collect();
    assert!(
        discovered.iter().all(|path| !mentions_results(path)),
        "discovered: {discovered:?}"
    );
}

#[test]
fn result_store_proof_stat_sweep_ignores_results_dir() {
    let (project, _db_dir, connection, _revision) = setup_project();
    let (_slot, _planted) = plant_store_files(project.path());
    let deadline = Deadline::after(process_limits().max_sync_wall_clock);

    let report = sweep(&connection, project.path(), &deadline).expect("sweep succeeds");

    let seen: Vec<&String> = report
        .candidates
        .iter()
        .chain(&report.deleted)
        .chain(&report.on_disk)
        .collect();
    assert!(
        seen.iter().all(|path| !mentions_results(path)),
        "sweep saw: {seen:?}"
    );
    assert!(report.unrepresentable.is_empty());
    assert!(report.on_disk.contains("a.rs") && report.on_disk.contains("b.rs"));
}

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

#[test]
fn result_store_proof_watcher_scope_excludes_results_dir() {
    // Canonical temp dir: real watcher events carry canonical paths.
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temp dir");
    let project = tempfile::tempdir_in(base).expect("project dir");
    std::fs::create_dir_all(project.path().join(".planning/slugaudit")).expect("activation dir");
    write(project.path(), "lib.rs", b"pub fn a() {}\n");
    let (_slot, _planted) = plant_store_files(project.path());
    let scope = WatchScope::compute(project.path());
    let results_dir: PathBuf = project.path().join(RESULTS);
    assert!(
        scope
            .watch_dirs
            .iter()
            .all(|dir| !dir.starts_with(project.path().join(".planning/slugaudit"))),
        "scope: {:?}",
        scope.watch_dirs
    );
    assert!(!scope.watch_dirs.contains(&results_dir));

    let watcher = WatchManager::with_watcher();
    let state = watcher.watch(project.path());
    if state.health() == WatcherHealth::Unavailable {
        return;
    }
    state.set_health(WatcherHealth::Healthy);
    write(project.path(), &format!("{RESULTS}/late-result.db"), b"x");
    write(project.path(), &format!("{RESULTS}/late.lock"), b"x");
    // Events arrive in order: once the control write is seen, the earlier
    // writes under the results directory have been delivered or dropped.
    write(project.path(), "control.rs", b"pub fn c() {}\n");

    assert!(
        wait_until(|| state.snapshot().dirty_paths.contains("control.rs")),
        "the control event must arrive; health: {:?}",
        state.health()
    );
    let snapshot = state.snapshot();
    let reached: Vec<&String> = snapshot
        .dirty_paths
        .iter()
        .chain(&snapshot.deleted_paths)
        .collect();
    assert!(
        reached.iter().all(|path| !mentions_results(path)),
        "events that reach reconcile: {reached:?}"
    );
}

/// Reconcile itself refuses the results directory: the planted store paths
/// are handed to it directly (as dirty paths, and the stray file as a
/// deletion), alongside a genuine new file.
#[test]
fn result_store_proof_reconcile_ignores_results_dir() {
    let (project, _db_dir, mut connection, revision) = setup_project();
    let (_slot, planted) = plant_store_files(project.path());
    write(project.path(), "c.rs", b"fn c() {}\n");
    let mut dirty: HashSet<String> = planted.iter().cloned().collect();
    dirty.insert("c.rs".to_owned());
    let on_disk: HashSet<String> = ["a.rs", "b.rs", "c.rs"].map(str::to_owned).into();
    let limits = *process_limits();
    let options = ReconcileOptions {
        limits,
        deadline: Deadline::after(limits.max_sync_wall_clock),
        rules: None,
    };

    reconcile_dirty_paths_with_deadline(
        &mut connection,
        project.path(),
        dirty,
        HashSet::new(),
        Some(&revision),
        &options,
        &on_disk,
    )
    .expect("reconcile succeeds");

    assert_eq!(
        stored_paths(&connection),
        ["a.rs", "b.rs", "c.rs"],
        "planted files: {planted:?}"
    );
}

#[test]
fn result_store_proof_incremental_sync_ignores_results_dir() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write(project.path(), "lib.rs", b"pub fn a() {}\n");
    let first = sync_project(&manager, &project);
    let (_slot, planted) = plant_store_files(project.path());
    write(project.path(), "lib.rs", b"pub fn a_longer_name() {}\n");

    let second = manager
        .ensure_current(&project.path().to_string_lossy(), &NoopProgressSink)
        .expect("incremental sync succeeds");

    assert_ne!(first.revision_id, second.revision_id, "the edit published");
    let connection = open_read_only(&second.database_path).expect("open the index");
    assert_eq!(
        stored_paths(&connection),
        ["lib.rs"],
        "planted files: {planted:?}"
    );
}
