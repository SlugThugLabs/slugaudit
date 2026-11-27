//! Paginate: cursors that expire and the result store limits.
use super::tests::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

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
    let half = one.checked_div(2).check("one / 2")?;
    let cap = one.checked_add(half).check("one + half")?;
    let (_project, path) = project()?;
    let manager = manager(cap);
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
        "remedy matches ResultExpired",
    )?;
    Ok(())
}

#[test]
fn paginate_full_store_refuses_a_new_result_and_keeps_active_ones_readable() -> TestResult {
    let one = one_result_bytes()?;
    let half = one.checked_div(2).check("one / 2")?;
    let cap = one.checked_add(half).check("one + half")?;
    let (_project, path) = project()?;
    let manager = manager(cap);
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
        &numbers(&second),
        &((10..20).collect::<Vec<_>>()),
        "numbers(&second) == (10..20)",
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

    let error = fresh(&manager, &path, 25, 10).err().check("unavailable")?;

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
        "remedy matches ResultStoreUnavailable",
    )?;
    Ok(())
}

#[test]
fn paginate_single_page_fast_path_skips_result_store_io() -> TestResult {
    let (project, path) = project()?;
    std::fs::write(
        project.path().join(".planning/slugaudit/results"),
        b"blocked",
    )
    .check("block results path to prove zero ResultStore disk I/O")?;
    let manager = manager(ROOMY);

    let served = fresh(&manager, &path, 5, 10).check("single-page result succeeds in memory")?;

    ensure(served.page.complete, "single page is complete")?;
    ensure_eq(&served.page.cursor, &None, "single page has no cursor")?;
    ensure_eq(
        &numbers(&served),
        &vec![0, 1, 2, 3, 4],
        "returns all five rows from memory",
    )?;
    ensure(
        manager
            .result_stores()
            .current()
            .check("store slot")?
            .is_none(),
        "ResultStore was never opened for a single-page result",
    )?;
    Ok(())
}

#[test]
fn paginate_continuation_serves_page_when_project_db_is_missing() -> TestResult {
    let (project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 20, 10).check("page one")?;
    let token = cursor_of(&first)?;
    crate::store::remove_database_files(&project.path().join(".planning/slugaudit/project.db"))
        .check("remove project.db")?;

    let second = next(&manager, TOOL, &token, 10).check("continuation still succeeds")?;

    ensure_eq(
        &second.page.revision_id,
        &first.page.revision_id,
        "keeps page one's stored revision_id",
    )?;
    ensure_eq(
        &second.page.current_disk_revision_id,
        &None,
        "missing project.db omits current_disk_revision_id without failing",
    )?;
    Ok(())
}
