//! `health`: the unreadable-database state, default vs verbose shape, and
//! multi-project `ResultStore` isolation.
use super::tests::{activated_temp_project, synced_manager};
use super::*;
use crate::sync::SourceSyncManager;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

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
        "database_read_failed is true",
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
            .check("answer[\"database\"]")?
            .get("file_count")
            .check("file_count")?,
        &1,
        "file_count == 1",
    )?;
    ensure(
        answer
            .get("database")
            .check("answer[\"database\"]")?
            .get("revision_id")
            .check("revision_id")?
            .is_string(),
        "revision_id is string",
    )?;
    Ok(())
}

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
            "result_store",
            "server",
            "system_memory",
            "system_memory_error",
        ],
        "verbose detail keys omit removed mechanic_cache",
    )?;
    ensure(
        detail
            .get("system_memory")
            .check("system_memory")?
            .get("total_bytes")
            .check("total_bytes")?
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "total_bytes > 0",
    )?;
    ensure(
        detail["system_memory_error"].is_null(),
        "detail[\"system_memory_error\"].is_null()",
    )?;
    ensure_eq(
        &keys(&detail["database"])?,
        &["parser_pack_version", "skipped_file_count"],
        "database detail keys",
    )?;
    ensure_eq(
        &detail["database_read_failed"],
        &false,
        "database_read_failed == false",
    )?;
    ensure(
        detail.get("database_read_error").is_none(),
        "database_read_error omitted when false",
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

#[test]
fn health_output_schema_exposes_typed_detail_definitions_not_free_form_maps() -> TestResult {
    let schema = serde_json::to_value(schemars::schema_for!(super::HealthResponse))
        .check("schema serializes")?;
    let defs = schema
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .check("HealthResponse schema defines $defs")?;

    for expected_def in [
        "HealthDetail",
        "DatabaseDetail",
        "ServerCounters",
        "ResultStoreState",
        "ResultClassState",
        "ResultSweepState",
        "SystemMemoryInfo",
    ] {
        ensure(
            defs.contains_key(expected_def),
            format!("HealthResponse schema $defs must include `{expected_def}`: {defs:?}"),
        )?;
    }
    ensure(
        !defs.contains_key("MechanicCacheState"),
        "MechanicCacheState was deleted under C5",
    )?;

    let detail_props = defs
        .get("HealthDetail")
        .and_then(|def| def.get("properties"))
        .and_then(serde_json::Value::as_object)
        .check("HealthDetail has typed properties")?;
    for prop in ["database", "server", "system_memory", "result_store"] {
        let prop_schema = detail_props
            .get(prop)
            .check(&format!("HealthDetail property `{prop}`"))?;
        let text = prop_schema.to_string();
        ensure(
            !text.contains("additionalProperties"),
            format!("HealthDetail.{prop} must not degrade to an untyped map: {text}"),
        )?;
    }
    Ok(())
}

#[test]
fn health_result_store_does_not_leak_across_projects() -> TestResult {
    let (_project_a, path_a) = activated_temp_project()?;
    let (_project_b, path_b) = activated_temp_project()?;
    let manager = SourceSyncManager::new();
    manager
        .ensure_current(&path_b, &crate::progress::NoopProgressSink)
        .check("sync project B")?;

    crate::tools::query(
        &Parameters(crate::tools::QueryRequest {
            path: Some(path_a.clone()),
            sql: "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<40) SELECT i, printf('%040d', i) AS pad FROM n".to_owned(),
            cursor: None,
            legacy_paging: crate::tools::LegacyPaging::default(),
            page_bytes: Some(400),
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("multi-page query on project A opens A's ResultStore")?;

    let health_a = health_json(&manager, &path_a, true)?;
    let store_a = health_a
        .get("detail")
        .and_then(|detail| detail.get("result_store"))
        .check("project A health detail carries result_store")?;
    ensure(
        !store_a.is_null(),
        "project A health reports project A's open ResultStore",
    )?;

    let health_b = health_json(&manager, &path_b, true)?;
    let store_b = health_b
        .get("detail")
        .and_then(|detail| detail.get("result_store"))
        .check("project B health detail carries result_store")?;
    ensure(
        store_b.is_null(),
        "project A's ResultStore must NOT leak into unpaged project B's health detail",
    )?;

    let health_global = super::health(
        &Parameters(super::HealthRequest {
            path: None,
            verbose: true,
        }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("global health")?
    .0;
    ensure(
        health_global
            .detail
            .check("verbose detail")?
            .result_store
            .is_none(),
        "global health without path reports no project ResultStore",
    )?;
    Ok(())
}
