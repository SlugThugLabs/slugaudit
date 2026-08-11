//! Tests for the database-backed half of the `health` tool:
//! `compute_project_state`, `compute_global_only_state`, and the
//! revision/parser-pack/file-count helpers — the branches that reach
//! into the project database. All use a local watcher-less manager so
//! health-visible state is deterministic (no async watcher events).

use crate::store;
use crate::sync::SourceSyncManager;
use crate::watch::WatcherHealth;
use std::fs;

fn activated_temp_project() -> (tempfile::TempDir, String) {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).expect("activation dir");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

fn synced_manager(project: &tempfile::TempDir, path: &str) -> SourceSyncManager {
    let manager = SourceSyncManager::new();
    manager
        .ensure_current(path, &crate::progress::NoopProgressSink)
        .expect("sync");
    assert!(
        project
            .path()
            .join(".planning/slugaudit/project.db")
            .exists(),
        "sync must create the project database"
    );
    manager
}

#[test]
fn project_state_populates_database_fields_for_an_active_project() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);

    let fields = super::compute_project_state(&path, &manager).expect("project state");

    assert_eq!(fields.watcher_health, Some(WatcherHealth::Unavailable));
    assert_eq!(fields.phase, super::HealthPhase::Unavailable);
    assert!(
        fields.revision_id.is_some(),
        "a revision must exist after sync"
    );
    assert!(fields.parser_pack_version.is_some());
    assert_eq!(fields.file_count, 1, "one indexed source file");
    assert_eq!(fields.pending_dirty, Some(0));
    assert_eq!(fields.pending_deleted, Some(0));
    assert!(fields.watcher_sequence.is_some());
    assert!(fields.last_verified_sequence.is_some());
}

#[test]
fn global_only_state_is_no_active_project_when_the_manager_is_empty() {
    let manager = SourceSyncManager::new();
    let fields = super::compute_global_only_state(&manager);
    assert_eq!(fields.phase, super::HealthPhase::NoActiveProject);
    assert_eq!(fields.watcher_health, None);
    assert_eq!(fields.pending_dirty, None);
    assert_eq!(fields.pending_deleted, None);
    assert_eq!(fields.watcher_sequence, None);
    assert_eq!(fields.last_verified_sequence, None);
    assert_eq!(fields.revision_id, None);
    assert_eq!(fields.parser_pack_version, None);
    assert_eq!(fields.file_count, -1);
}

#[test]
fn global_only_state_reports_watcher_fields_when_a_project_is_registered() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);

    let fields = super::compute_global_only_state(&manager);
    assert_eq!(fields.phase, super::HealthPhase::Unavailable);
    assert_eq!(fields.watcher_health, Some(WatcherHealth::Unavailable));
    assert_eq!(fields.pending_dirty, Some(0));
    assert_eq!(fields.pending_deleted, Some(0));
    // The global-only path never opens the database:
    assert_eq!(fields.revision_id, None);
    assert_eq!(fields.parser_pack_version, None);
    assert_eq!(fields.file_count, -1);
}

#[test]
fn revision_helpers_reflect_the_published_database() {
    let (project, path) = activated_temp_project();
    synced_manager(&project, &path);
    let database = project.path().join(".planning/slugaudit/project.db");
    let mut connection = store::open_read_only(&database).expect("open");

    let revision = super::current_revision_id(&mut connection);
    assert!(revision.is_some(), "a published revision must be visible");

    let version = super::current_parser_pack_version(&mut connection);
    assert_eq!(version.as_deref(), Some(crate::parse::PACK_VERSION));

    assert_eq!(super::count_files(&mut connection).expect("file count"), 1);
}

#[test]
fn revision_helpers_return_none_on_a_fresh_database() {
    let directory = tempfile::tempdir().expect("db dir");
    let mut connection =
        store::open_read_write(&directory.path().join("project.db")).expect("open fresh db");
    assert_eq!(
        super::current_revision_id(&mut connection),
        None,
        "no revision before any publish"
    );
    assert_eq!(super::current_parser_pack_version(&mut connection), None);
    assert_eq!(super::count_files(&mut connection).expect("query"), 0);
}

/// A project that is enabled (marker exists) but has never been synced
/// must degrade gracefully: no error, watcher fields None, DB fields
/// None/-1, phase `NoActiveProject` — the marker exists but this server
/// has nothing active for it.
#[test]
fn an_enabled_but_never_synced_project_degrades_gracefully() {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).expect("activation dir");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();

    let manager = SourceSyncManager::new();
    let fields = super::compute_project_state(&path, &manager).expect("no error");

    assert_eq!(fields.phase, super::HealthPhase::NoActiveProject);
    assert_eq!(fields.watcher_health, None);
    assert_eq!(fields.pending_dirty, None);
    assert_eq!(fields.revision_id, None);
    assert_eq!(fields.parser_pack_version, None);
    assert_eq!(fields.file_count, -1);
}

/// The core read-only guarantee: calling `health` with a path must never
/// create a revision or move `last_sync` — it reports, it does not sync.
/// Before this behavior was fixed, the path branch ran `ensure_current`,
/// which would publish on a modified project.
#[test]
fn health_with_a_path_never_triggers_a_sync() {
    let (project, path) = activated_temp_project();
    let manager = SourceSyncManager::new();
    manager
        .ensure_current(&path, &crate::progress::NoopProgressSink)
        .expect("initial sync");
    let revision_after_sync = manager.last_sync_unix_seconds();
    assert!(revision_after_sync > 0);

    // Modify the project on disk — a sync would notice and publish.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify source");

    super::compute_project_state(&path, &manager).expect("health read");

    let database = project.path().join(".planning/slugaudit/project.db");
    let connection = store::open_read_only(&database).expect("open");
    let revision_count: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("revision count");
    assert_eq!(
        revision_count, 1,
        "health must not publish a new revision for the modified file"
    );
    assert_eq!(
        manager.last_sync_unix_seconds(),
        revision_after_sync,
        "health must not stamp the last-sync timestamp"
    );
}

#[test]
fn health_with_an_active_project_path_reports_db_fields() {
    let (project, path) = activated_temp_project();
    // Deliberate coupling to the process-global manager: this is the real
    // production entry point (`health` with a path goes through the same
    // `ensure_synced`). The registered temp project leaves a watch entry
    // in the global manager until its (deleted) root is pruned by a later
    // event; every other test that reads the global manager is path-keyed,
    // so this coupling is safe today — assert only path-independent fields
    // here so it stays that way.
    crate::tools::context::ensure_synced(&path, &crate::progress::NoopProgressSink)
        .expect("register with the process manager");

    let request = crate::tools::HealthRequest { path: Some(path) };
    let response = crate::tools::health::health(
        &rmcp::handler::server::wrapper::Parameters(request),
        &crate::progress::NoopProgressSink,
    )
    .expect("health with a path succeeds");

    let inner = response.0;
    assert!(inner.revision_id.is_some());
    assert!(inner.parser_pack_version.is_some());
    assert_eq!(inner.file_count, 1);
    assert!(
        inner.last_sync_unix_seconds > 0,
        "ensure_current must stamp the last-sync timestamp"
    );
    let _ = project;
}
