//! `health`: the unreadable-database state and the default versus verbose answer.
use super::*;

/// A database that exists but cannot be read must report `unknown`, never
/// `importing` ("the first sync never succeeded") for a project whose first
/// sync did succeed.
#[test]
fn an_unreadable_database_after_a_successful_sync_reports_unknown() -> TestResult {
    let (project, path) = activated_temp_project()?;
    let _manager = synced_manager(&project, &path)?;
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"this is not a sqlite database").check("corrupt the database")?;

    let response = super::super::health(
        &Parameters(super::super::HealthRequest {
            path: Some(path),
            verbose: true,
        }),
        &crate::progress::NoopProgressSink,
        &SourceSyncManager::new(),
    )
    .check("an unreadable database degrades to fields, not an error")?;

    ensure_eq(
        &response.0.phase,
        &HealthPhase::Unknown,
        "response.0.phase == HealthPhase::Unknown",
    )?;
    ensure(
        response
            .0
            .detail
            .as_ref()
            .check("a verbose answer carries detail")?
            .database_read_failed,
        "response .0 .detail .as_ref() .check(\"a verbose answer carries detail\")? .database_read_failed",
    )?;
    let error = response
        .0
        .detail
        .as_ref()
        .check("a verbose answer carries detail")?
        .database_read_error
        .as_deref()
        .check("the response must carry why the database read failed")?;
    ensure(
        error.contains("not a database"),
        "error.contains(\"not a database\")",
    )?;
    ensure_eq(&response.0.database, &None, "response.0.database == None")?;
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
    let response = super::super::health(
        &Parameters(super::super::HealthRequest {
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

    ensure_eq(
        &keys(&answer)?,
        &["database", "phase", "remedy"],
        "keys(&answer)? == [\"database\", \"phase\", \"remedy\"]",
    )?;
    ensure_eq(
        &keys(answer.get("database").check("answer[\"database\"]")?)?,
        &["file_count", "revision_id"],
        "keys(&answer[\"database\"])? == [\"file_count\", \"revision_id\"]",
    )?;
    ensure_eq(
        answer
            .get("database")
            .check("answer[\"database\"][\"file_count\"]")?
            .get("file_count")
            .check("answer[\"database\"][\"file_count\"]")?,
        &1,
        "answer[\"database\"][\"file_count\"] == 1",
    )?;
    ensure(
        answer
            .get("database")
            .check("answer[\"database\"][\"revision_id\"]")?
            .get("revision_id")
            .check("answer[\"database\"][\"revision_id\"]")?
            .is_string(),
        "answer[\"database\"][\"revision_id\"].is_string()",
    )?;
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

    let detail = answer.get("detail").check("answer[\"detail\"]")?;
    ensure_eq(
        &keys(detail)?,
        &[
            "database",
            "database_read_failed",
            "mechanic_cache",
            "result_store",
            "server",
            "system_memory",
            "system_memory_error",
        ],
        "keys(detail)? == [ \"database\", \"database_read_failed\", \"mechanic_cache\", \"result_store\", \"server\", \"system_memory\", \"system_memory_error\" ]",
    )?;
    ensure(
        detail
            .get("system_memory")
            .check("detail[\"system_memory\"][\"total_bytes\"]")?
            .get("total_bytes")
            .check("detail[\"system_memory\"][\"total_bytes\"]")?
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "detail[\"system_memory\"][\"total_bytes\"] .as_u64() .is_some_and(|bytes| bytes > 0)",
    )?;
    ensure(
        detail["system_memory_error"].is_null(),
        "detail[\"system_memory_error\"].is_null()",
    )?;
    ensure_eq(
        &keys(&detail["database"])?,
        &["parser_pack_version", "skipped_file_count"],
        "keys(&detail[\"database\"])? == [\"parser_pack_version\", \"skipped_file_count\"]",
    )?;
    ensure_eq(
        &detail["database_read_failed"],
        &false,
        "detail[\"database_read_failed\"] == false",
    )?;
    ensure(
        detail.get("database_read_error").is_none(),
        "detail.get(\"database_read_error\").is_none()",
    )?;
    ensure(
        detail["result_store"].is_null(),
        "detail[\"result_store\"].is_null()",
    )?;
    for field in ["phase", "remedy", "database"] {
        ensure_eq(
            answer.get(field).check("answer field")?,
            plain.get(field).check("plain field")?,
            "answer[field] == plain[field]",
        )?;
    }
    Ok(())
}
