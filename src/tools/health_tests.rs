//! Tests for the `health` MCP tool.
//!
//! The crate re-exports both a `health` *function* (`crate::tools::health`)
//! and a `health` *module* (`crate::tools::health` as a path), so any
//! `use super::health::{...}` is ambiguous. We therefore reference the
//! function and module items by their full path inside test bodies,
//! importing only the types we need.

use crate::store;
use crate::sync::SourceSyncManager;
use crate::tools::HealthRequest;
use crate::tools::health::{HealthPhase, ToolCounters};
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult};
use std::fs;

fn req(path: Option<&str>) -> Parameters<HealthRequest> {
    Parameters(HealthRequest {
        path: path.map(str::to_owned),
        verbose: true,
    })
}

#[test]
fn tool_counters_record_bumps_call_and_total_counts() {
    let before = ToolCounters::snapshot();
    let after = ToolCounters::record_call(true, 17);
    assert_eq!(after.call_count, before.call_count + 1);
    assert_eq!(
        after.error_count, before.error_count,
        "successful call does NOT bump error_count",
    );
    assert_eq!(after.total_ms, before.total_ms + 17);
}

#[test]
fn tool_counters_record_bumps_error_count_on_error() {
    let before = ToolCounters::snapshot();
    let after = ToolCounters::record_call(false, 3);
    assert_eq!(after.call_count, before.call_count + 1);
    assert_eq!(after.error_count, before.error_count + 1);
    assert_eq!(after.total_ms, before.total_ms + 3);
}

#[test]
fn tool_counters_always_return_non_decreasing_after_a_bump() {
    // The returned snapshot must reflect this call's increments, not a
    // stale value loaded before the call lands.
    let snap_calls_before = ToolCounters::snapshot().call_count;
    let snap_after = ToolCounters::record_call(true, 0);
    assert!(snap_after.call_count > snap_calls_before);
}

#[test]
fn health_rejects_a_path_over_the_byte_cap_before_resolving_it() -> TestResult {
    let path = "a".repeat(32 * 1024 + 1);

    let error = crate::tools::health::health(
        &req(Some(&path)),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("an over-cap path is refused")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert_eq!(error.message, "path exceeds 32768 bytes");
    Ok(())
}

#[test]
fn health_with_an_unrelated_path_returns_the_resolution_error() -> TestResult {
    // No active project for this path — `resolve_project` fails with a
    // find-project-root error. The health tool surfaces such errors rather
    // than degrading silently: a path argument is a deliberate operator
    // query, not a fallback we run rate-limited.
    let missing = "/definitely/not/a/real/project/that/exists";
    let error = crate::tools::health::health(
        &req(Some(missing)),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .err()
    .check("health should surface the underlying error for a bad path")?;

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error.message.contains("failed to resolve project root"),
        "the message must name resolution rather than the server: {}",
        error.message
    );
    assert!(
        error.message.contains("No such file or directory"),
        "the underlying cause must survive: {}",
        error.message
    );
    Ok(())
}

#[test]
fn read_database_is_unreadable_when_the_database_cannot_be_opened() -> TestResult {
    use crate::tools::health::DatabaseRead;
    let directory = tempfile::tempdir().check("temp dir")?;
    let db = directory.path().join("project.db");
    std::fs::write(&db, b"this is not a sqlite database").check("corrupt fixture")?;
    // An unopenable file is exactly the failure `read_database` must degrade
    // on, not propagate: the whole `database` group goes absent and the
    // failure is logged. It is "unreadable", never "never synced": the file
    // is right there.
    assert_eq!(
        crate::tools::health::read_database(&db),
        DatabaseRead::Unreadable,
        "an unopenable database is absent, never a half-filled group"
    );
    Ok(())
}

#[test]
fn read_database_is_never_synced_when_no_database_exists_yet() -> TestResult {
    use crate::tools::health::DatabaseRead;
    let directory = tempfile::tempdir().check("temp dir")?;
    assert_eq!(
        crate::tools::health::read_database(&directory.path().join("never-synced.db")),
        DatabaseRead::NeverSynced
    );
    Ok(())
}

/// The health database WARNs name the real database path (`path=`) and the
/// raw source error, not a redacted `database_id`. Covers the inspect and the
/// read failures; the control line proves the capture was live.
#[test]
fn health_database_warns_log_the_real_path_and_source_error() -> TestResult {
    let directory = tempfile::tempdir().check("dir")?;
    let file = directory.path().join("health_real_path_blocker");
    std::fs::write(&file, b"x").check("write file")?;
    let inspect_path = file.join("project.db");
    let read_path = directory.path().join("health_real_path_read.db");
    let connection = crate::store::open_read_write(&read_path).check("open the database")?;
    connection
        .execute_batch("DROP TABLE revisions")
        .check("remove the revisions table")?;

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for health_real_path");
        super::read_database(&inspect_path);
        super::read_database(&read_path);
    });

    assert!(
        logs.contains("capture liveness control for health_real_path"),
        "the capture must be live: {logs}"
    );
    let inspect = logs
        .lines()
        .find(|line| line.contains("failed to inspect the project database path"))
        .check(&format!("inspect record missing:\n{logs}"))?;
    assert!(
        inspect.contains(&format!(" path={} ", inspect_path.display())),
        "inspect WARN must carry the real path:\n{inspect}"
    );
    assert!(
        inspect.contains(" error="),
        "source error missing:\n{inspect}"
    );
    let read = logs
        .lines()
        .find(|line| line.contains("failed to read the project database"))
        .check(&format!("read record missing:\n{logs}"))?;
    assert!(
        read.contains(&format!(" path={} ", read_path.display())),
        "read WARN must carry the real path:\n{read}"
    );
    assert!(
        read.contains("no such table"),
        "source error missing:\n{read}"
    );
    assert!(
        !logs.contains("database_id="),
        "the redacted database id is gone:\n{logs}"
    );
    Ok(())
}

fn activated_temp_project() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activation dir")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

fn synced_manager(project: &tempfile::TempDir, path: &str) -> TestResult<SourceSyncManager> {
    let manager = SourceSyncManager::new();
    manager
        .ensure_current(path, &crate::progress::NoopProgressSink)
        .check("sync")?;
    assert!(
        project
            .path()
            .join(".planning/slugaudit/project.db")
            .exists(),
        "sync must create the project database"
    );
    Ok(manager)
}

#[test]
fn project_state_populates_database_fields_for_an_active_project() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let _manager = synced_manager(&project, &path)?;

    let fields = super::compute_project_state(&path).check("project state")?;

    assert_eq!(fields.phase, HealthPhase::SteadyState);
    assert_eq!(fields.remedy, None);
    assert!(!fields.database_read_failed);
    let database = fields.database.as_ref().check("database group")?;
    assert!(
        database.revision_id.is_some(),
        "a revision must exist after sync"
    );
    assert!(database.parser_pack_version.is_some());
    assert_eq!(database.file_count, 1, "one indexed source file");
    assert_eq!(database.skipped_file_count, 0, "nothing was skipped");
    Ok(())
}

#[test]
fn global_only_state_is_no_active_project() {
    let fields = super::compute_global_only_state();
    assert_eq!(fields.phase, HealthPhase::NoActiveProject);
    assert_eq!(fields.database, None, "no path never opens a database");
}

/// The core read-only guarantee: calling `health` with a path must never
/// create a revision or move `last_sync` — it reports, it does not sync.
#[test]
fn health_with_a_path_never_triggers_a_sync() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let manager = synced_manager(&project, &path)?;
    let revision_after_sync = manager.last_sync_unix_seconds();
    assert!(revision_after_sync > 0);

    // Modify the project on disk — a sync would notice and publish.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .check("modify source")?;

    super::compute_project_state(&path).check("health read")?;

    let database = project.path().join(".planning/slugaudit/project.db");
    let connection = store::open_existing(&database).check("open")?;
    let revision_count: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .check("revision count")?;
    assert_eq!(
        revision_count, 1,
        "health must not publish a new revision for the modified file"
    );
    assert_eq!(
        manager.last_sync_unix_seconds(),
        revision_after_sync,
        "health must not stamp the last-sync timestamp"
    );
    Ok(())
}

#[test]
fn health_with_an_active_project_path_reports_db_fields() -> TestResult {
    let (project, path) = activated_temp_project()?;
    // Production path: `ensure_synced` (via `SlugAuditServer`) and `health`
    // (same server.rs instance) share the same `SourceSyncManager`, so the
    // last-sync timestamp the first one stamps is visible to the second.
    let manager = synced_manager(&project, &path)?;

    let request = crate::tools::HealthRequest {
        path: Some(path),
        verbose: true,
    };
    let response = crate::tools::health::health(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("health with a path succeeds")?;

    let inner = response.0;
    let database = inner.database.as_ref().check("database group")?;
    assert!(database.revision_id.is_some());
    assert_eq!(database.file_count, 1);
    let detail = inner
        .detail
        .as_ref()
        .check("a verbose answer carries detail")?;
    let database_detail = detail.database.as_ref().check("database detail group")?;
    assert!(database_detail.parser_pack_version.is_some());
    assert!(
        detail.server.last_sync_unix_seconds > 0,
        "ensure_current must stamp the last-sync timestamp"
    );
    let _ = project;
    Ok(())
}

/// A `path` with no activation marker is a *state*, not a failure: a
/// diagnostic tool that fails when there is nothing to diagnose is useless.
/// The answer must say `disabled` and carry the same remedy every tool's
/// `project_not_enabled` error carries, so a caller can treat both as one
/// instruction.
#[test]
fn health_with_a_path_outside_any_enabled_project_reports_disabled() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();

    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path),
            verbose: false,
        }),
        &crate::progress::NoopProgressSink,
        &SourceSyncManager::new(),
    )
    .check("a disabled path is an answer, not an error")?;

    assert_eq!(response.0.phase, HealthPhase::Disabled);
    assert_eq!(response.0.database, None);
    let remedy = response
        .0
        .remedy
        .as_deref()
        .check("a disabled answer carries the remedy")?;
    assert!(remedy.contains("project_control"), "{remedy}");
    assert!(remedy.contains("\"action\":\"on\""), "{remedy}");
    assert!(remedy.contains("<project directory>"), "{remedy}");
    Ok(())
}

/// The no-`path` case names no project, but it is still the state a cold
/// caller lands in: it must say nothing is active and how to start.
#[test]
fn health_without_a_path_still_names_the_way_to_enable_a_project() -> TestResult {
    let response = super::health(
        &Parameters(super::HealthRequest {
            path: None,
            verbose: false,
        }),
        &crate::progress::NoopProgressSink,
        &SourceSyncManager::new(),
    )
    .check("health never fails without a path")?;

    assert_eq!(response.0.phase, HealthPhase::NoActiveProject);
    assert!(response.0.remedy.is_some());
    Ok(())
}

/// Once a project is actually being served the remedy disappears: the field
/// means "no project is served for this answer", nothing else.
#[test]
fn health_reports_no_remedy_for_an_active_project() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let manager = synced_manager(&project, &path)?;

    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path),
            verbose: false,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("health succeeds for an active project")?;

    assert_eq!(response.0.remedy, None);
    assert!(
        response
            .0
            .database
            .as_ref()
            .check("database group")?
            .revision_id
            .is_some()
    );
    Ok(())
}

/// A database that exists but cannot be read must report `unknown`, never
/// `importing` ("the first sync never succeeded") for a project whose first
/// sync did succeed.
#[test]
fn an_unreadable_database_after_a_successful_sync_reports_unknown() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let _manager = synced_manager(&project, &path)?;
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"this is not a sqlite database").check("corrupt the database")?;

    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path),
            verbose: true,
        }),
        &crate::progress::NoopProgressSink,
        &SourceSyncManager::new(),
    )
    .check("an unreadable database degrades to fields, not an error")?;

    assert_eq!(
        response.0.phase,
        HealthPhase::Unknown,
        "an unreadable database must not claim the first sync never succeeded"
    );
    assert!(
        response
            .0
            .detail
            .as_ref()
            .check("a verbose answer carries detail")?
            .database_read_failed,
        "the response must carry that the database read failed"
    );
    assert_eq!(response.0.database, None);
    Ok(())
}

/// Sorted object keys of one JSON value, for asserting a response's shape.
fn keys(value: &serde_json::Value) -> TestResult<Vec<&str>> {
    let mut keys: Vec<&str> = value
        .as_object()
        .check("a JSON object")?
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    Ok(keys)
}

fn health_json(
    manager: &SourceSyncManager,
    path: &str,
    verbose: bool,
) -> TestResult<serde_json::Value> {
    let response = super::health(
        &Parameters(super::HealthRequest {
            path: Some(path.to_owned()),
            verbose,
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("health succeeds for an active project")?;
    serde_json::to_value(&response.0).check("health serialises")
}

/// The default answer is what a model acts on and nothing else: the phase,
/// the remedy, and the revision with its file count.
#[test]
fn health_without_verbose_carries_only_the_fields_a_caller_acts_on() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let manager = synced_manager(&project, &path)?;

    let answer = health_json(&manager, &path, false)?;

    assert_eq!(keys(&answer)?, ["database", "phase", "remedy"]);
    assert_eq!(keys(&answer["database"])?, ["file_count", "revision_id"]);
    assert_eq!(answer["database"]["file_count"], 1);
    assert!(answer["database"]["revision_id"].is_string(), "{answer}");
    Ok(())
}

/// `verbose: true` adds the operator view under `detail` and leaves the
/// default fields exactly as they were.
#[test]
fn health_with_verbose_adds_the_operator_detail_beside_the_default_fields() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let manager = synced_manager(&project, &path)?;
    let plain = health_json(&manager, &path, false)?;

    let answer = health_json(&manager, &path, true)?;

    let detail = &answer["detail"];
    assert_eq!(
        keys(detail)?,
        [
            "database",
            "database_read_failed",
            "mechanic_cache",
            "result_store",
            "server",
            "system_memory",
            "system_memory_error"
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
        keys(&detail["database"])?,
        ["parser_pack_version", "skipped_file_count"]
    );
    assert_eq!(detail["database_read_failed"], false);
    assert!(
        detail["result_store"].is_null(),
        "no paged result was made yet"
    );
    for field in ["phase", "remedy", "database"] {
        assert_eq!(
            answer[field], plain[field],
            "`{field}` changed with verbose"
        );
    }
    Ok(())
}
