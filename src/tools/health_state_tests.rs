//! Tests for the database-backed half of the `health` tool:
//! `compute_project_state`, `compute_global_only_state`, and the
//! revision/parser-pack/file-count helpers — the branches that reach
//! into the project database. All use a local watcher-less manager so
//! health-visible state is deterministic (no async watcher events).

use crate::store;
use crate::sync::SourceSyncManager;
use crate::watch::WatcherHealth;
use rmcp::handler::server::wrapper::Parameters;
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

    let watcher = fields.watcher.as_ref().expect("watcher group");
    assert_eq!(watcher.health, WatcherHealth::Unavailable);
    assert_eq!(fields.phase, super::HealthPhase::Unavailable);
    assert_eq!(watcher.pending_dirty, 0);
    assert_eq!(watcher.pending_deleted, 0);
    let database = fields.database.as_ref().expect("database group");
    assert!(
        database.revision_id.is_some(),
        "a revision must exist after sync"
    );
    assert!(database.parser_pack_version.is_some());
    assert_eq!(database.file_count, 1, "one indexed source file");
    assert_eq!(database.skipped_file_count, 0, "nothing was skipped");
}

#[test]
fn global_only_state_is_no_active_project_when_the_manager_is_empty() {
    let manager = SourceSyncManager::new();
    let fields = super::compute_global_only_state(&manager);
    assert_eq!(fields.phase, super::HealthPhase::NoActiveProject);
    assert_eq!(
        fields.watcher, None,
        "no project registered means no watcher group at all"
    );
    assert_eq!(
        fields.database, None,
        "the global path never opens a database"
    );
}

#[test]
fn global_only_state_reports_watcher_fields_when_a_project_is_registered() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);

    let fields = super::compute_global_only_state(&manager);
    assert_eq!(fields.phase, super::HealthPhase::Unavailable);
    let watcher = fields.watcher.as_ref().expect("watcher group");
    assert_eq!(watcher.health, WatcherHealth::Unavailable);
    assert_eq!(watcher.pending_dirty, 0);
    assert_eq!(watcher.pending_deleted, 0);
    // The global-only path never opens the database:
    assert_eq!(fields.database, None);
}

/// Without a `path` there is no project root to open a database with, so a
/// healthy watcher with nothing pending says nothing about whether a
/// revision exists. It must not claim `importing` ("the first sync has
/// never succeeded") or `steady_state` ("a current revision exists").
#[test]
fn global_only_state_with_a_healthy_watcher_reports_an_unknown_phase() {
    let (project, _path) = activated_temp_project();
    let manager = SourceSyncManager::new();
    let state = manager.activate(project.path()).expect("activate");
    state.set_health(WatcherHealth::Healthy);

    let fields = super::compute_global_only_state(&manager);

    assert_eq!(fields.phase, super::HealthPhase::Unknown);
    assert_eq!(
        fields.watcher.as_ref().map(|watcher| watcher.health),
        Some(WatcherHealth::Healthy),
        "the watcher's own health is still reported"
    );
    assert_eq!(
        fields.database, None,
        "the global path never opens a database"
    );
}

/// A watcher that is healthy but has unreconciled events has a phase the
/// watcher alone can name: the next call reconciles. No revision claim is
/// needed for `restoring`.
#[test]
fn global_only_state_with_pending_events_reports_restoring() {
    let (project, _path) = activated_temp_project();
    let manager = SourceSyncManager::new();
    let state = manager.activate(project.path()).expect("activate");
    state.set_health(WatcherHealth::Healthy);
    state.mark_dirty("lib.rs".to_owned());

    let fields = super::compute_global_only_state(&manager);

    assert_eq!(fields.phase, super::HealthPhase::Restoring);
}

#[test]
fn revision_helpers_reflect_the_published_database() {
    let (project, path) = activated_temp_project();
    synced_manager(&project, &path);
    let database = project.path().join(".planning/slugaudit/project.db");
    let mut connection = store::open_read_only(&database).expect("open");

    let (revision, version) =
        super::read_revision_fields(&mut connection).expect("read revision fields");
    assert!(revision.is_some(), "a published revision must be visible");
    assert_eq!(version.as_deref(), Some(crate::parse::PACK_VERSION));

    assert_eq!(super::count_files(&mut connection).expect("file count"), 1);
}

/// The behaviour the two helpers this replaced got wrong: they ended in
/// `.ok().flatten()`, so a query error and "nothing published yet" both
/// arrived as `None` and `health` reported a broken database as an
/// unpublished one. A missing table must reach the caller as an error.
#[test]
fn a_failed_revision_query_is_an_error_not_a_missing_revision() {
    let mut connection = rusqlite::Connection::open_in_memory().expect("in-memory db");
    let error = super::read_revision_fields(&mut connection)
        .expect_err("a missing revisions table must error, not read as no revision");
    assert!(
        error.to_string().contains("no such table"),
        "the underlying cause must survive: {error}"
    );
}

#[test]
fn revision_helpers_return_none_on_a_fresh_database() {
    let directory = tempfile::tempdir().expect("db dir");
    let mut connection =
        store::open_read_write(&directory.path().join("project.db")).expect("open fresh db");
    assert_eq!(
        super::read_revision_fields(&mut connection).expect("read revision fields"),
        (None, None),
        "no revision before any publish"
    );
    assert_eq!(super::count_files(&mut connection).expect("query"), 0);
}

/// A project that is enabled (marker exists) but has never been synced
/// must degrade gracefully: no error, both per-project groups absent, phase
/// `NoActiveProject` — the marker exists but this server has nothing active
/// for it.
#[test]
fn an_enabled_but_never_synced_project_degrades_gracefully() {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit")).expect("activation dir");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();

    let manager = SourceSyncManager::new();
    let fields = super::compute_project_state(&path, &manager).expect("no error");

    assert_eq!(fields.phase, super::HealthPhase::NoActiveProject);
    assert_eq!(fields.watcher, None, "no watcher was ever registered");
    assert_eq!(fields.database, None, "nothing was ever published");
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
    // Production path: `ensure_synced` (via `SlugAuditServer`) and `health`
    // (same server.rs instance) share the same `SourceSyncManager`, so the
    // last-sync timestamp the first one stamps is visible to the second.
    // In test, all of `ensure_synced_no_progress`, `health`, and the watch
    // state they read from must use the *same* instance — bind once and
    // pass by reference, matching the production `SlugAuditServer::clone()`
    // shape that derives a new manager reference from the same `Arc`s-holding
    // internals.
    let manager = crate::sync::SourceSyncManager::with_watcher();
    crate::tools::context::ensure_synced_no_progress(&path, &manager)
        .expect("ensure_current succeeds via the test manager");

    let request = crate::tools::HealthRequest {
        path: Some(path),
        verbose: true,
    };
    let response = crate::tools::health::health(
        &rmcp::handler::server::wrapper::Parameters(request),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("health with a path succeeds");

    let inner = response.0;
    let database = inner.database.as_ref().expect("database group");
    assert!(database.revision_id.is_some());
    assert_eq!(database.file_count, 1);
    let detail = inner
        .detail
        .as_ref()
        .expect("a verbose answer carries detail");
    let database_detail = detail.database.as_ref().expect("database detail group");
    assert!(database_detail.parser_pack_version.is_some());
    assert!(
        detail.server.last_sync_unix_seconds > 0,
        "ensure_current must stamp the last-sync timestamp"
    );
    let _ = project;
}

/// A `path` with no activation marker is a *state*, not a failure: a
/// diagnostic tool that fails when there is nothing to diagnose is useless.
/// The answer must say `disabled` and carry the same remedy every tool's
/// `project_not_enabled` error carries, so a caller can treat both as one
/// instruction.
#[test]
fn health_with_a_path_outside_any_enabled_project_reports_disabled() {
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();

    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path),
            verbose: false,
        }),
        &crate::progress::NoopProgressSink,
        &SourceSyncManager::new(),
    )
    .expect("a disabled path is an answer, not an error");

    assert_eq!(response.0.phase, super::HealthPhase::Disabled);
    assert_eq!(response.0.watcher, None);
    assert_eq!(response.0.database, None);
    let remedy = response
        .0
        .remedy
        .as_deref()
        .expect("a disabled answer carries the remedy");
    assert!(remedy.contains("project_control"), "{remedy}");
    assert!(remedy.contains("\"action\":\"on\""), "{remedy}");
    assert!(remedy.contains("<project directory>"), "{remedy}");
}

/// The no-`path` case names no project, but it is still the state a cold
/// caller lands in: it must say nothing is active and how to start.
#[test]
fn health_without_a_path_still_names_the_way_to_enable_a_project() {
    let response = super::health(
        &Parameters(super::HealthRequest {
            path: None,
            verbose: false,
        }),
        &crate::progress::NoopProgressSink,
        &SourceSyncManager::new(),
    )
    .expect("health never fails without a path");

    assert_eq!(response.0.phase, super::HealthPhase::NoActiveProject);
    assert!(response.0.remedy.is_some());
}

/// Once a project is actually being served the remedy disappears: the field
/// means "no project is served for this answer", nothing else.
#[test]
fn health_reports_no_remedy_for_an_active_project() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);

    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path),
            verbose: false,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("health succeeds for an active project");

    assert_eq!(response.0.remedy, None);
    assert!(
        response
            .0
            .database
            .as_ref()
            .expect("database group")
            .revision_id
            .is_some()
    );
}

#[test]
fn published_project_without_watcher_is_reported_as_indexed_unwatched() {
    let (project, path) = activated_temp_project();
    synced_manager(&project, &path);
    let fresh_server_manager = SourceSyncManager::new();

    let fields = super::compute_project_state(&path, &fresh_server_manager)
        .expect("read existing published database");

    assert_eq!(fields.phase, super::HealthPhase::IndexedUnwatched);
    let database = fields.database.as_ref().expect("database group");
    assert!(
        database.revision_id.is_some(),
        "indexed project retains revision"
    );
    assert_eq!(database.file_count, 1);
    assert_eq!(fields.watcher, None, "no watcher was registered");
    assert_eq!(fields.remedy, None);
}

/// `Path::exists` folds every stat error into `false`, so an uninspectable
/// database path read as "never synced" with no log line. A path below a
/// regular file fails with ENOTDIR, which is not an absent database.
#[test]
fn an_uninspectable_database_path_is_logged_not_read_as_never_synced() {
    let directory = tempfile::tempdir().expect("dir");
    let file = directory.path().join("not_a_directory");
    fs::write(&file, b"x").expect("write file");

    let (state, logs) = crate::store::test_capture::capture_warns(|| {
        super::read_database(&file.join("project.db"))
    });

    assert_eq!(
        state,
        crate::tools::health::DatabaseRead::Unreadable,
        "an uninspectable path is not the same fact as a never-synced project"
    );
    assert!(
        logs.contains("failed to inspect the project database path"),
        "logs: {logs}"
    );
}

/// The database opens but its queries fail (here: the schema's `revisions`
/// table is gone). The whole database group goes absent and the failure is
/// logged, so a half-read answer is never returned and an operator can tell
/// an unreadable database from a never-synced one.
#[test]
fn a_failed_database_query_is_logged_and_reported_as_absent() {
    let directory = tempfile::tempdir().expect("db dir");
    let database = directory.path().join("project.db");
    let connection = store::open_read_write(&database).expect("open the database");
    connection
        .execute_batch("DROP TABLE revisions")
        .expect("remove the revisions table");

    let (state, logs) =
        crate::store::test_capture::capture_warns(|| super::read_database(&database));

    assert_eq!(
        state,
        crate::tools::health::DatabaseRead::Unreadable,
        "an unreadable database is absent, never partial"
    );
    assert!(
        logs.contains("failed to read the project database"),
        "the query failure must be logged as itself: {logs}"
    );
    assert!(
        logs.contains("no such table"),
        "the underlying cause must survive: {logs}"
    );
}

/// The defect this pins: a healthy watcher plus a database that exists but
/// cannot be read used to reach `derive_phase` as `revision_id = None`,
/// which reports `Importing` — "the first sync has never succeeded" — for a
/// project whose first sync did succeed. The answer must say the revision
/// is unknown and carry the read failure as its own field.
#[test]
fn an_unreadable_database_after_a_successful_sync_reports_unknown_not_importing() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);
    let state = manager.activate(project.path()).expect("activate");
    state.set_health(WatcherHealth::Healthy);
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"this is not a sqlite database").expect("corrupt the database");

    let response = crate::tools::health::health(
        &Parameters(super::HealthRequest {
            path: Some(path),
            verbose: true,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("an unreadable database degrades to fields, not an error");

    assert_eq!(
        response.0.phase,
        super::HealthPhase::Unknown,
        "an unreadable database must not claim the first sync never succeeded"
    );
    assert!(
        response
            .0
            .detail
            .as_ref()
            .expect("a verbose answer carries detail")
            .database_read_failed,
        "the response must carry that the database read failed"
    );
    assert_eq!(response.0.database, None);
    assert_eq!(
        response.0.watcher.as_ref().map(|watcher| watcher.health),
        Some(WatcherHealth::Healthy),
        "the watcher's own health is still reported"
    );
}

/// Sorted object keys of one JSON value, for asserting a response's shape.
fn keys(value: &serde_json::Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

fn health_json(manager: &SourceSyncManager, path: &str, verbose: bool) -> serde_json::Value {
    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path.to_owned()),
            verbose,
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .expect("health succeeds for an active project");
    serde_json::to_value(&response.0).expect("health serialises")
}

/// The default answer is what a model acts on and nothing else: the phase,
/// the remedy, the watcher's health, and the revision with its file count.
#[test]
fn health_without_verbose_carries_only_the_fields_a_caller_acts_on() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);

    let answer = health_json(&manager, &path, false);

    assert_eq!(keys(&answer), ["database", "phase", "remedy", "watcher"]);
    assert_eq!(keys(&answer["watcher"]), ["health"]);
    assert_eq!(keys(&answer["database"]), ["file_count", "revision_id"]);
    assert_eq!(answer["database"]["file_count"], 1);
    assert!(answer["database"]["revision_id"].is_string(), "{answer}");
}

/// `verbose: true` adds the operator view under `detail` and leaves the
/// default fields exactly as they were.
#[test]
fn health_with_verbose_adds_the_operator_detail_beside_the_default_fields() {
    let (project, path) = activated_temp_project();
    let manager = synced_manager(&project, &path);
    let plain = health_json(&manager, &path, false);

    let answer = health_json(&manager, &path, true);

    let detail = &answer["detail"];
    assert_eq!(
        keys(detail),
        [
            "database",
            "database_read_failed",
            "mechanic_cache",
            "result_store",
            "server",
            "system_memory",
            "system_memory_error",
            "watcher"
        ]
    );
    assert!(
        detail["system_memory"]["total_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "detected memory is reported as detected: {detail}"
    );
    assert!(
        detail["system_memory_error"].is_null(),
        "no detection error when memory was detected: {detail}"
    );
    assert_eq!(
        keys(&detail["watcher"]),
        [
            "last_verified_sequence",
            "pending_deleted",
            "pending_dirty",
            "sequence"
        ]
    );
    assert_eq!(
        keys(&detail["database"]),
        ["parser_pack_version", "skipped_file_count"]
    );
    assert_eq!(detail["database_read_failed"], false);
    assert!(
        detail["result_store"].is_null(),
        "no paged result was made yet"
    );
    for field in ["phase", "remedy", "watcher", "database"] {
        assert_eq!(
            answer[field], plain[field],
            "`{field}` changed with verbose"
        );
    }
}
