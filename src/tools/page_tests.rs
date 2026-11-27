//! Unit tests for [`super`] (`LegacyPaging`, `Page`, `PageLimit`, `PageSize`).

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn legacy_paging_refuses_an_offset_with_the_cursor_remedy() -> TestResult {
    let legacy: LegacyPaging =
        serde_json::from_value(serde_json::json!({ "offset": 5 })).check("parses")?;

    let error = legacy.refuse().check_err("offset is removed")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    let data = error.data.check("a payload")?;
    ensure_eq(
        data.get("code").check("data[\"code\"]")?,
        &"paging_parameter_removed",
        "data[\"code\"] == \"paging_parameter_removed\"",
    )?;
    ensure(
        data.get("remedy")
            .check("data[\"remedy\"]")?
            .as_str()
            .check("remedy")?
            .contains("cursor"),
        "remedy mentions cursor",
    )?;
    Ok(())
}

#[test]
fn legacy_paging_refuses_an_expected_revision_with_the_cursor_remedy() -> TestResult {
    let legacy: LegacyPaging =
        serde_json::from_value(serde_json::json!({ "expected_revision": "r1" })).check("parses")?;

    let error = legacy.refuse().check_err("expected_revision is removed")?;

    let data = error.data.check("a payload")?;
    ensure_eq(
        data.get("code").check("data[\"code\"]")?,
        &"paging_parameter_removed",
        "data[\"code\"] == \"paging_parameter_removed\"",
    )?;
    ensure(
        data.get("remedy")
            .check("data[\"remedy\"]")?
            .as_str()
            .check("remedy")?
            .contains("cursor"),
        "remedy mentions cursor",
    )?;
    Ok(())
}

#[test]
fn legacy_paging_accepts_a_request_with_neither_field() -> TestResult {
    let legacy: LegacyPaging = serde_json::from_value(serde_json::json!({})).check("parses")?;

    legacy.refuse().check("nothing to refuse")?;
    Ok(())
}

#[test]
fn legacy_paging_flattened_into_a_request_is_read_but_not_in_its_schema() -> TestResult {
    #[derive(Deserialize, JsonSchema)]
    struct Probe {
        #[serde(default)]
        path: Option<String>,
        #[serde(flatten, default)]
        #[schemars(skip)]
        legacy_paging: LegacyPaging,
    }

    let probe: Probe =
        serde_json::from_value(serde_json::json!({ "path": "p", "offset": 1 })).check("parses")?;
    let schema = serde_json::to_string(&schemars::schema_for!(Probe)).check("schema")?;

    ensure_eq(
        &probe.path.as_deref(),
        &Some("p"),
        "probe.path.as_deref() == Some(\"p\")",
    )?;
    probe.legacy_paging.refuse().check_err("offset was sent")?;
    ensure(!schema.contains("offset"), &schema)?;
    ensure(!schema.contains("expected_revision"), &schema)?;
    ensure(!schema.contains("legacy_paging"), &schema)?;
    Ok(())
}

#[test]
fn a_page_serializes_exactly_the_seven_shared_field_names() -> TestResult {
    let page = Page {
        database_discard: None,
        returned: 2,
        total: Some(5),
        complete: false,
        cursor: None,
        completeness: None,
        page_bytes: 100,
        bytes_used: 90,
        limited_by: PageLimit::Ceiling,
        revision_id: "rev-1".to_owned(),
        current_disk_revision_id: None,
    };

    let value = serde_json::to_value(&page).check("a page serializes")?;

    ensure_eq(
        &value,
        &serde_json::json!({
            "returned": 2,
            "total": 5,
            "complete": false,
            "page_bytes": 100,
            "bytes_used": 90,
            "limited_by": "ceiling",
            "revision_id": "rev-1",
        }),
        "minimal Page serializes the seven required keys",
    )?;
    Ok(())
}

#[test]
fn a_last_page_serializes_unknown_total_as_json_null_and_has_no_next_offset() -> TestResult {
    let page = Page {
        database_discard: None,
        returned: 1,
        total: None,
        complete: true,
        cursor: None,
        completeness: None,
        page_bytes: 100,
        bytes_used: 10,
        limited_by: PageLimit::None,
        revision_id: "rev-1".to_owned(),
        current_disk_revision_id: None,
    };

    let value = serde_json::to_value(&page).check("a page serializes")?;

    ensure_eq(
        value.get("total").check("value[\"total\"]")?,
        &serde_json::Value::Null,
        "value[\"total\"] == serde_json::Value::Null",
    )?;
    ensure(
        !value
            .as_object()
            .check("an object")?
            .contains_key("next_offset"),
        "the pre-cursor field is gone from the wire",
    )?;
    Ok(())
}

#[test]
fn every_page_limit_serializes_as_its_snake_case_name() -> TestResult {
    let names = [
        PageLimit::None,
        PageLimit::Limit,
        PageLimit::PageBytes,
        PageLimit::Ceiling,
        PageLimit::MatchCap,
        PageLimit::ScanWindow,
        PageLimit::TimeBudget,
        PageLimit::QueryMatchLimit,
    ]
    .into_iter()
    .map(|limit| serde_json::to_value(limit).check("a limit serializes"))
    .collect::<TestResult<Vec<_>>>()?;

    ensure_eq(
        &names,
        &[
            "none",
            "limit",
            "page_bytes",
            "ceiling",
            "match_cap",
            "scan_window",
            "time_budget",
            "query_match_limit",
        ]
        .map(serde_json::Value::from),
        "all PageLimit variants serialize in snake_case",
    )?;
    Ok(())
}

#[test]
fn an_omitted_page_size_resolves_to_the_default() -> TestResult {
    let limits = ResourceLimits::default();

    let size = PageSize::resolve(None, &limits).check("default resolves")?;

    ensure_eq(
        &size.bytes,
        &limits.default_page_bytes,
        "size.bytes == limits.default_page_bytes",
    )?;
    ensure(!size.at_ceiling, "!size.at_ceiling")?;
    ensure_eq(
        &size.byte_cut(),
        &PageLimit::PageBytes,
        "size.byte_cut() == PageLimit::PageBytes",
    )?;
    Ok(())
}

#[test]
fn a_page_size_above_or_at_the_ceiling_is_clamped_and_reports_the_ceiling() -> TestResult {
    let limits = ResourceLimits {
        max_query_response_bytes: 1_000,
        ..ResourceLimits::default()
    };

    for requested in [Some(5_000), Some(1_000)] {
        let size = PageSize::resolve(requested, &limits).check("clamps")?;
        ensure_eq(
            &(size.bytes, size.byte_cut()),
            &(1_000, PageLimit::Ceiling),
            "clamped or exact ceiling reports Ceiling",
        )?;
    }

    let below = PageSize::resolve(Some(999), &limits).check("below ceiling")?;
    ensure_eq(
        &(below.bytes, below.byte_cut()),
        &(999, PageLimit::PageBytes),
        "below ceiling reports PageBytes",
    )?;
    Ok(())
}

#[test]
fn a_configured_default_above_the_ceiling_is_clamped_and_reported() -> TestResult {
    let limits = ResourceLimits {
        max_query_response_bytes: 100,
        default_page_bytes: 400,
        ..ResourceLimits::default()
    };

    let size = PageSize::resolve(None, &limits).check("clamps")?;

    ensure_eq(
        &(size.bytes, size.byte_cut()),
        &(100, PageLimit::Ceiling),
        "(size.bytes, size.byte_cut()) == (100, PageLimit::Ceiling)",
    )?;
    Ok(())
}

#[test]
fn a_zero_page_size_is_refused_not_substituted() -> TestResult {
    let error = PageSize::resolve(Some(0), &ResourceLimits::default()).check_err("zero fails")?;

    ensure(
        error.message.contains("page_bytes"),
        format!("{}", error.message),
    )?;
    ensure_eq(
        &error.data.as_ref().and_then(|d| d["code"].as_str()),
        &Some("invalid_input"),
        "zero page_bytes carries invalid_input code",
    )?;
    Ok(())
}
