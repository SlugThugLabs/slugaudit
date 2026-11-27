//! Unit tests for [`super::SlugAuditServer`], resource serving, and MCP input/output schemas.

use super::handlers::input_schema;
use super::*;
use crate::tools;
use rmcp::handler::server::common::schema_for_input;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn matches_rmcp<T: schemars::JsonSchema + std::any::Any>() -> TestResult {
    let checked = schema_for_input::<T>().check(std::any::type_name::<T>())?;
    ensure_eq(&input_schema::<T>(), &checked, std::any::type_name::<T>())?;
    Ok(())
}

#[test]
fn every_input_schema_matches_rmcp_and_has_an_object_root() -> TestResult {
    matches_rmcp::<tools::ReportRequest>()?;
    matches_rmcp::<tools::LeadsRequest>()?;
    matches_rmcp::<tools::MechanicRequest>()?;
    matches_rmcp::<tools::QueryRequest>()?;
    matches_rmcp::<tools::StructureRequest>()?;
    matches_rmcp::<tools::FindingRequest>()?;
    matches_rmcp::<tools::FindingReadRequest>()?;
    matches_rmcp::<tools::FindingClearRequest>()?;
    matches_rmcp::<tools::ProjectControlRequest>()?;
    matches_rmcp::<tools::HealthRequest>()?;
    Ok(())
}

#[test]
fn the_initialize_instructions_stay_under_their_size_budget() -> TestResult {
    let characters = INSTRUCTIONS.chars().count();

    ensure(
        characters < 5_500,
        format!("INSTRUCTIONS is {characters} characters; the budget is 5500"),
    )?;
    Ok(())
}

#[test]
fn advertised_instructions_and_tool_surface_never_mention_host_env_or_config_file() -> TestResult {
    ensure(
        !INSTRUCTIONS.contains("SLUGAUDIT_") && !INSTRUCTIONS.contains("config.json"),
        "INSTRUCTIONS leaked SLUGAUDIT_* or config.json",
    )?;
    let surface_json = serde_json::to_string(&tool_surface()).check("serialize tool_surface")?;
    ensure(
        !surface_json.contains("SLUGAUDIT_") && !surface_json.contains("config.json"),
        format!("tool_surface leaked SLUGAUDIT_* or config.json: {surface_json}"),
    )?;
    Ok(())
}

#[test]
fn the_query_tool_description_names_every_evidence_kind_the_schema_allows() -> TestResult {
    let schema = include_str!("store/schema.sql");
    let check = schema
        .split("CHECK (kind IN (")
        .nth(1)
        .and_then(|rest| rest.split("))").next())
        .check("the evidence kind CHECK list")?;
    let kinds: Vec<&str> = check
        .split(',')
        .map(|kind| kind.trim().trim_matches('\''))
        .collect();
    let surface = tool_surface();
    let query = surface
        .iter()
        .find(|tool| tool.name == "query")
        .check("the query tool is listed")?;

    let description = query
        .description
        .as_deref()
        .check("query has a description")?;

    ensure_eq(&kinds.len(), &8, format!("{kinds:?}"))?;
    for kind in kinds {
        ensure(
            description.contains(kind),
            format!("`{kind}` missing: {description}"),
        )?;
    }
    Ok(())
}

#[test]
fn read_resource_reports_an_unknown_uri_as_resource_not_found() -> TestResult {
    let error =
        read_resource_contents("slugaudit://nope").check_err("an unknown URI is not served")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::RESOURCE_NOT_FOUND,
        format!("{error:?}"),
    )?;
    ensure_eq(
        &error.message,
        &"unknown resource `slugaudit://nope`",
        "error.message == \"unknown resource `slugaudit://nope`\"",
    )?;
    Ok(())
}

#[test]
fn read_resource_refuses_a_uri_over_the_cap_without_echoing_it() -> TestResult {
    let uri = "u".repeat(MAX_RESOURCE_URI_BYTES.checked_add(1).check("cap + 1")?);

    let error = read_resource_contents(&uri).check_err("an over-cap URI is refused")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        format!("{error:?}"),
    )?;
    ensure_eq(
        &error.message,
        &"resource uri exceeds 8192 bytes",
        "error.message == \"resource uri exceeds 8192 bytes\"",
    )?;
    ensure(
        !error.message.contains("uuuu"),
        format!(
            "an oversized URI must not be echoed into the message: {}",
            error.message
        ),
    )?;
    Ok(())
}
