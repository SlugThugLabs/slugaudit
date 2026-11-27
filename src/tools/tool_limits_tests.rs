//! Unit tests for [`super`] (`resolve_limits`, `limits_for_root`, `deadline_after`).

use super::*;
use crate::tools::test_support;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn a_poisoned_lock_maps_to_a_restart_error_without_the_file_hint() -> TestResult {
    let mutex = std::sync::Mutex::new(0_u8);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> TestResult {
        let _guard = mutex.lock().check("lock the fresh mutex")?;
        std::panic::resume_unwind(Box::new("simulated panic inside the critical section"));
    }));
    ensure(unwound.is_err(), "the holder must have unwound")?;
    let poisoned =
        crate::util::lock(&mutex, "limits_test_mutex").check_err("a poisoned mutex is an error")?;

    let error = map_limits_error(crate::project::ProjectLimitsError::Lock(poisoned));

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure_eq(
        &error.message,
        &"mutex limits_test_mutex was poisoned by a panic in an earlier holder; \
         restart the SlugAudit server",
        "poisoned mutex message names the mutex and advises server restart",
    )?;
    Ok(())
}

#[test]
fn resolve_limits_rejects_a_path_with_no_active_project() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;

    let error = resolve_limits(directory.path().to_str().check("utf-8 path")?)
        .check_err("a path with no project root must fail, not fall back")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    let code = error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(|code| code.as_str());
    ensure_eq(
        &code,
        &Some("project_not_enabled"),
        "code == Some(\"project_not_enabled\")",
    )?;
    Ok(())
}

#[test]
fn resolve_limits_returns_limits_for_an_active_project() -> TestResult {
    let project = test_support::activated_project("a.rs", b"fn a() {}")?;

    let limits = resolve_limits(project.path().to_str().check("utf-8 path")?)
        .check("an active project resolves")?;

    ensure(limits.max_file_bytes > 0, "limits.max_file_bytes > 0")?;
    ensure(
        deadline_after(Duration::from_secs(1)).is_some(),
        "finite budget yields a deadline",
    )?;
    Ok(())
}

#[test]
fn fail_closed_a_malformed_global_config_fails_the_call_naming_the_file() -> TestResult {
    let project = test_support::activated_project("a.rs", b"fn a() {}")?;
    let home = tempfile::tempdir().check("temp home")?;
    let config = home.path().join("bad-settings.json");
    std::fs::write(&config, "{ not json").check("write config")?;
    crate::project::set_global_config_path_for_test(Some(config.clone()));

    let error = resolve_limits(project.path().to_str().check("utf-8 path")?)
        .check_err("a malformed config must fail the call, not fall back to defaults")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure(
        error.message.contains(config.to_str().check("utf-8 path")?),
        format!("{}", error.message),
    )?;
    Ok(())
}
