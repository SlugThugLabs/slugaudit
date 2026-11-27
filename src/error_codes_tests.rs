use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::collections::BTreeSet;

#[test]
fn every_code_is_unique_snake_case_and_free_of_host_config_mentions() -> TestResult {
    let mut seen = BTreeSet::new();
    for code in ErrorCode::ALL {
        let text = code.as_str();
        ensure(
            !text.is_empty()
                && text.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_')
                && seen.insert(text),
            format!("invalid or duplicate wire code: {text}"),
        )?;
        let remedy = code.remedy();
        ensure(
            !remedy.is_empty() && !remedy.contains("SLUGAUDIT_") && !remedy.contains("config.json"),
            format!("{text} remedy invalid or leaked host config: {remedy}"),
        )?;
    }
    Ok(())
}

#[test]
fn the_budget_payload_quotes_profile_budgets_and_session_action() -> TestResult {
    use crate::model::{AuditProfile, limits_for_profile};
    let p = budget_payload(Some(120_000)).check("spent budget carries payload")?;
    ensure_eq(
        p.get("code").check("code")?,
        &"sync_budget_exhausted",
        "code",
    )?;
    let remedy = p.get("remedy").and_then(Value::as_str).check("remedy")?;
    for (profile, name) in [(AuditProfile::Audit, "audit"), (AuditProfile::Lean, "lean")] {
        let secs = limits_for_profile(profile)
            .check("profile")?
            .max_sync_wall_clock
            .as_secs();
        ensure(
            remedy.contains(&format!("{name} profile ({secs} s)")),
            format!("missing {secs} s for {name}: {remedy}"),
        )?;
    }
    ensure(remedy.contains("project_control"), "names project_control")?;
    ensure(budget_payload(None).is_none(), "non-budget gets None")?;
    Ok(())
}

#[test]
fn store_errors_map_to_their_wire_codes_and_static_remedies() -> TestResult {
    use crate::store::ResultStoreError as E;
    let unavailable = E::Unavailable {
        context: "open the result store",
        source: Box::new(std::io::Error::other("unlogged detail /x")),
    };
    for (error, code, internal) in [
        (E::Unknown, Some(ErrorCode::UnknownCursor), false),
        (E::Expired, Some(ErrorCode::ResultExpired), false),
        (
            E::Full { active_count: 3 },
            Some(ErrorCode::ResultStoreFull),
            false,
        ),
        (E::TooLarge, Some(ErrorCode::ResultTooLarge), false),
        (unavailable, Some(ErrorCode::ResultStoreUnavailable), true),
        (E::Corrupt { context: "x" }, None, true),
    ] {
        let mapped = store_error(&error);
        let expected = code.map(|c| json!({"code": c.as_str(), "remedy": c.remedy()}));
        ensure_eq(&mapped.data, &expected, format!("{error:?}"))?;
        ensure_eq(
            &(mapped.code == rmcp::model::ErrorCode::INTERNAL_ERROR),
            &internal,
            "kind",
        )?;
        ensure(!mapped.message.contains("unlogged"), "redacted source")?;
    }
    Ok(())
}
