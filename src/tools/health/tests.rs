//! Tests for the `health` MCP tool.
//!
//! The crate re-exports both a `health` *function* (`crate::tools::health`)
//! and a `health` *module* (`crate::tools::health` as a path), so any
//! `use super::health::{...}` is ambiguous. We therefore reference the
//! function and module items by their full path inside test bodies,
//! importing only the types we need.

use crate::store;
use crate::sync::SourceSyncManager;
use crate::tools::health::HealthPhase;
use crate::tools::{HealthRequest, ToolCounters};
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

fn req(path: Option<&str>) -> Parameters<HealthRequest> {
    Parameters(HealthRequest {
        path: path.map(str::to_owned),
        verbose: true,
    })
}

#[test]
fn tool_counters_record_bumps_call_and_total_counts() -> TestResult {
    let before = ToolCounters::snapshot();
    let after = ToolCounters::record_call(true, 17);
    ensure_eq(
        &after.call_count,
        &(before.call_count + 1),
        "after.call_count == before.call_count + 1",
    )?;
    ensure_eq(
        &after.error_count,
        &before.error_count,
        "successful call does NOT bump error_count",
    )?;
    ensure_eq(
        &after.total_ms,
        &(before.total_ms + 17),
        "after.total_ms == before.total_ms + 17",
    )?;
    Ok(())
}

#[test]
fn tool_counters_record_bumps_error_count_on_error() -> TestResult {
    let before = ToolCounters::snapshot();
    let after = ToolCounters::record_call(false, 3);
    ensure_eq(
        &after.call_count,
        &(before.call_count + 1),
        "after.call_count == before.call_count + 1",
    )?;
    ensure_eq(
        &after.error_count,
        &(before.error_count + 1),
        "after.error_count == before.error_count + 1",
    )?;
    ensure_eq(
        &after.total_ms,
        &(before.total_ms + 3),
        "after.total_ms == before.total_ms + 3",
    )?;
    Ok(())
}

#[test]
fn tool_counters_always_return_non_decreasing_after_a_bump() -> TestResult {
    // The returned snapshot must reflect this call's increments, not a
    // stale value loaded before the call lands.
    let snap_calls_before = ToolCounters::snapshot().call_count;
    let snap_after = ToolCounters::record_call(true, 0);
    ensure(
        snap_after.call_count > snap_calls_before,
        "snap_after.call_count > snap_calls_before",
    )?;
    Ok(())
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

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &"path exceeds 32768 bytes",
        "error.message == \"path exceeds 32768 bytes\"",
    )?;
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

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure(
        error.message.contains("failed to resolve project root"),
        format!(
            "the message must name resolution rather than the server: {}",
            error.message
        ),
    )?;
    ensure(
        error.message.contains("No such file or directory"),
        format!("the underlying cause must survive: {}", error.message),
    )?;
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
    let read = crate::tools::health::read_database(&db);
    let DatabaseRead::Unreadable(error) = read else {
        return Err(format!(
            "an unopenable database is unreadable, never a half-filled group: {read:?}"
        ))
        .check("unexpected outcome");
    };
    ensure(
        error.starts_with("reading the project database:")
            || error.starts_with("opening the project database read-only:"),
        "error.starts_with(\"reading the project database:\") || error.starts_with(\"opening the project database read-only:\")",
    )?;
    ensure(
        error.contains("not a database"),
        "error.contains(\"not a database\")",
    )?;
    Ok(())
}

#[test]
fn read_database_is_never_synced_when_no_database_exists_yet() -> TestResult {
    use crate::tools::health::DatabaseRead;
    let directory = tempfile::tempdir().check("temp dir")?;
    ensure_eq(
        &crate::tools::health::read_database(&directory.path().join("never-synced.db")),
        &DatabaseRead::NeverSynced,
        "crate::tools::health::read_database(&directory.path().join(\"never-synced.db\")) == DatabaseRead::NeverSynced",
    )?;
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
    })?;

    ensure(
        logs.contains("capture liveness control for health_real_path"),
        "logs.contains(\"capture liveness control for health_real_path\")",
    )?;
    let inspect = logs
        .lines()
        .find(|line| line.contains("step=\"inspecting the project database path\""))
        .check(&format!("inspect record missing:\n{logs}"))?;
    ensure(
        inspect.contains(&format!(" path={} ", inspect_path.display())),
        "inspect.contains(&format!(\" path={} \", inspect_path.display()))",
    )?;
    ensure(inspect.contains(" error="), "inspect.contains(\" error=\")")?;
    let read = logs
        .lines()
        .find(|line| line.contains("step=\"reading the project database\""))
        .check(&format!("read record missing:\n{logs}"))?;
    ensure(
        read.contains(&format!(" path={} ", read_path.display())),
        "read.contains(&format!(\" path={} \", read_path.display()))",
    )?;
    ensure(
        read.contains("no such table"),
        "read.contains(\"no such table\")",
    )?;
    ensure(
        !logs.contains("database_id="),
        "!logs.contains(\"database_id=\")",
    )?;
    Ok(())
}

pub(super) fn activated_temp_project() -> TestResult<(tempfile::TempDir, String)> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activation dir")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

pub(super) fn synced_manager(
    project: &tempfile::TempDir,
    path: &str,
) -> TestResult<SourceSyncManager> {
    let manager = SourceSyncManager::new();
    manager
        .ensure_current(path, &crate::progress::NoopProgressSink)
        .check("sync")?;
    ensure(
        project
            .path()
            .join(".planning/slugaudit/project.db")
            .exists(),
        "project .path() .join(\".planning/slugaudit/project.db\") .exists()",
    )?;
    Ok(manager)
}

#[test]
fn project_state_populates_database_fields_for_an_active_project() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let _manager = synced_manager(&project, &path)?;

    let fields = super::compute_project_state(&path).check("project state")?;

    ensure_eq(
        &fields.phase,
        &HealthPhase::SteadyState,
        "fields.phase == HealthPhase::SteadyState",
    )?;
    ensure_eq(&fields.remedy, &None, "fields.remedy == None")?;
    ensure_eq(
        &fields.database_read_error,
        &None,
        "fields.database_read_error == None",
    )?;
    let database = fields.database.as_ref().check("database group")?;
    database
        .revision_id
        .as_ref()
        .check("a revision must exist after sync")?;
    database
        .parser_pack_version
        .as_ref()
        .check("`database.parser_pack_version` is present")?;
    ensure_eq(&database.file_count, &1, "database.file_count == 1")?;
    ensure_eq(
        &database.skipped_file_count,
        &0,
        "database.skipped_file_count == 0",
    )?;
    Ok(())
}

#[test]
fn global_only_state_is_no_active_project() -> TestResult {
    let fields = super::compute_global_only_state();
    ensure_eq(
        &fields.phase,
        &HealthPhase::NoActiveProject,
        "fields.phase == HealthPhase::NoActiveProject",
    )?;
    ensure_eq(&fields.database, &None, "fields.database == None")?;
    Ok(())
}

/// The core read-only guarantee: calling `health` with a path must never
/// create a revision or move `last_sync` — it reports, it does not sync.
#[test]
fn health_with_a_path_never_triggers_a_sync() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let manager = synced_manager(&project, &path)?;
    let revision_after_sync = manager.last_sync_unix_seconds();
    ensure(revision_after_sync > 0, "revision_after_sync > 0")?;

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
    ensure_eq(&revision_count, &1, "revision_count == 1")?;
    ensure_eq(
        &manager.last_sync_unix_seconds(),
        &revision_after_sync,
        "manager.last_sync_unix_seconds() == revision_after_sync",
    )?;
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
    database
        .revision_id
        .as_ref()
        .check("`database.revision_id` is present")?;
    ensure_eq(&database.file_count, &1, "database.file_count == 1")?;
    let detail = inner
        .detail
        .as_ref()
        .check("a verbose answer carries detail")?;
    let database_detail = detail.database.as_ref().check("database detail group")?;
    database_detail
        .parser_pack_version
        .as_ref()
        .check("`database_detail.parser_pack_version` is present")?;
    ensure(
        detail.server.last_sync_unix_seconds > 0,
        "detail.server.last_sync_unix_seconds > 0",
    )?;
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

    ensure_eq(
        &response.0.phase,
        &HealthPhase::Disabled,
        "response.0.phase == HealthPhase::Disabled",
    )?;
    ensure_eq(&response.0.database, &None, "response.0.database == None")?;
    let remedy = response
        .0
        .remedy
        .as_deref()
        .check("a disabled answer carries the remedy")?;
    ensure(
        remedy.contains("project_control"),
        "remedy.contains(\"project_control\")",
    )?;
    ensure(
        remedy.contains("\"action\":\"on\""),
        "remedy.contains(\"\\\"action\\\":\\\"on\\\"\")",
    )?;
    ensure(
        remedy.contains("<project directory>"),
        "remedy.contains(\"<project directory>\")",
    )?;
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

    ensure_eq(
        &response.0.phase,
        &HealthPhase::NoActiveProject,
        "response.0.phase == HealthPhase::NoActiveProject",
    )?;
    response
        .0
        .remedy
        .as_ref()
        .check("`response.0.remedy` is present")?;
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

    ensure_eq(&response.0.remedy, &None, "response.0.remedy == None")?;
    response
        .0
        .database
        .as_ref()
        .check("database group")?
        .revision_id
        .as_ref()
        .check("a revision exists after sync")?;
    Ok(())
}
