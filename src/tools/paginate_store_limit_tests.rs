//! Paginate: cursors that expire and the result store limits.
use super::*;

/// Bytes one 20-row demo result takes in the store.
fn one_result_bytes() -> TestResult<u64> {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    fresh(&manager, &path, 20, 10).check("probe result")?;
    Ok(manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("store")?
        .stats()
        .check("read the store counters")?
        .admitted_bytes)
}

#[test]
fn paginate_reclaimed_result_continues_as_result_expired() -> TestResult {
    let one = one_result_bytes()?;
    let (_project, path) = project()?;
    let manager = manager(one + one / 2);
    let first = fresh(&manager, &path, 20, 10).check("page one")?;
    let stale = cursor_of(&first)?;
    next(&manager, TOOL, &stale, 10).check("delivers the result")?;

    fresh(&manager, &path, 20, 10).check("a new result reclaims the delivered one")?;
    let error = next(&manager, TOOL, &stale, 10).err().check("reclaimed")?;

    ensure_eq(
        &code_of(&error)?,
        &"result_expired",
        "code_of(&error)? == \"result_expired\"",
    )?;
    ensure_eq(
        &error.data.as_ref().check("payload")?["remedy"],
        &crate::error_codes::ErrorCode::ResultExpired.remedy(),
        "error.data.as_ref().check(\"payload\")?[\"remedy\"] == crate::error_codes::ErrorCode::ResultExpired.remedy()",
    )?;
    Ok(())
}

#[test]
fn paginate_full_store_refuses_a_new_result_and_keeps_active_ones_readable() -> TestResult {
    let one = one_result_bytes()?;
    let (_project, path) = project()?;
    let manager = manager(one + one / 2);
    let first = fresh(&manager, &path, 20, 10).check("page one")?;

    let error = fresh(&manager, &path, 20, 10)
        .err()
        .check("store is full")?;

    ensure_eq(
        &code_of(&error)?,
        &"result_store_full",
        "code_of(&error)? == \"result_store_full\"",
    )?;
    ensure(
        error.message.contains("1 active"),
        format!("{}", error.message),
    )?;
    let second = next(&manager, TOOL, &cursor_of(&first)?, 10).check("still readable")?;
    ensure_eq(
        &numbers(&second)?,
        &((10..20).collect::<Vec<_>>()),
        "numbers(&second)? == (10..20).collect::<Vec<_>>()",
    )?;
    Ok(())
}

#[test]
fn paginate_result_over_the_whole_budget_is_result_too_large_with_no_entry() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(4096);

    let error = fresh(&manager, &path, 50, 10).err().check("too large")?;

    ensure_eq(
        &code_of(&error)?,
        &"result_too_large",
        "code_of(&error)? == \"result_too_large\"",
    )?;
    let stats = manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("store")?
        .stats()
        .check("read the store counters")?;
    ensure_eq(&stats.admitted_bytes, &0, "stats.admitted_bytes == 0")?;
    ensure_eq(&stats.active.results, &0, "stats.active.results == 0")?;
    Ok(())
}

#[test]
fn paginate_unusable_store_is_result_store_unavailable() -> TestResult {
    let (project, path) = project()?;
    std::fs::write(
        project.path().join(".planning/slugaudit/results"),
        b"not a dir",
    )
    .check("block the results directory")?;
    let manager = manager(ROOMY);

    let error = fresh(&manager, &path, 5, 10).err().check("unavailable")?;

    ensure_eq(
        &code_of(&error)?,
        &"result_store_unavailable",
        "code_of(&error)? == \"result_store_unavailable\"",
    )?;
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure_eq(
        &error.data.as_ref().check("payload")?["remedy"],
        &crate::error_codes::ErrorCode::ResultStoreUnavailable.remedy(),
        "error.data.as_ref().check(\"payload\")?[\"remedy\"] == crate::error_codes::ErrorCode::ResultStoreUnavailable.remedy()",
    )?;
    Ok(())
}
