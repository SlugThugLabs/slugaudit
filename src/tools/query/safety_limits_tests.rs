//! Engine limit and amplifier refusals of the query tool.
use super::safety_tests::project;
use super::*;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::time::Duration;

/// `query` under `limits`, whose `max_file_bytes` and `max_query_value_bytes`
/// are both `file_bytes`, and the value-length bound that gives.
fn ask_bounded(
    project: &tempfile::TempDir,
    sql: &str,
    file_bytes: usize,
) -> TestResult<(Result<QueryResponse, ErrorData>, i32)> {
    let limits = ResourceLimits {
        max_file_bytes: file_bytes as u64,
        max_query_value_bytes: file_bytes,
        ..ResourceLimits::default()
    };
    let bound = super::exec::engine_limits(&limits)
        .check("engine limits")?
        .value_length;
    let request = Parameters(QueryRequest {
        path: Some(project.path().to_string_lossy().into_owned()),
        sql: sql.to_owned(),
        cursor: None,
        legacy_paging: LegacyPaging::default(),
        page_bytes: None,
    });
    let manager = crate::sync::SourceSyncManager::default();
    let result = query_with_limits(
        &request,
        &limits,
        &crate::progress::NoopProgressSink,
        &manager,
    );
    Ok((result.map(|Json(response)| response), bound))
}

/// Unbounded, the amplifier chain grows to gigabytes. Under a 3 MiB value-length bound
/// it, and each step of the same chain just above the bound, fails quickly
/// with SQLite's "too big" as `invalid_params`; the bound itself evaluates.
#[test]
fn the_zeroblob_hex_replace_amplifier_fails_fast_with_a_too_big_error() -> TestResult {
    let project = project()?;
    let bound = ask_bounded(&project, "SELECT 1 AS warm", 1024 * 1024)?.1;
    let started = std::time::Instant::now();

    for sql in [
        "SELECT length(replace(replace(hex(zeroblob(250000000)),'0','00'),'00','000')) AS n"
            .to_owned(),
        format!("SELECT length(zeroblob({})) AS n", bound + 1),
        format!("SELECT length(hex(zeroblob({}))) AS n", bound / 2 + 1),
    ] {
        let error = ask_bounded(&project, &sql, 1024 * 1024)?
            .0
            .check_err("over the bound")?;
        ensure_eq(&error.code, &rmcp::model::ErrorCode::INVALID_PARAMS, &sql)?;
        ensure(
            error.message.contains("too big"),
            format!("{sql}: {}", error.message),
        )?;
    }
    let at_bound = ask_bounded(
        &project,
        &format!("SELECT length(zeroblob({bound})) AS n"),
        1024 * 1024,
    )?
    .0;

    ensure_eq(
        &at_bound
            .check("exactly the bound")?
            .rows
            .first()
            .check("at_bound.check(\"exactly the bound\")?.rows[0]")?
            .get("n")
            .check("at_bound.check(\"exactly the bound\")?.rows[0][\"n\"]")?,
        &i64::from(bound),
        "at_bound.check(\"exactly the bound\")?.rows[0][\"n\"] == i64::from(bound)",
    )?;
    ensure(
        started.elapsed() < Duration::from_secs(5),
        format!("{:?}", started.elapsed()),
    )?;
    Ok(())
}

/// SQLite checks the value-length bound when it reads a stored column too.
/// Stored content is valid UTF-8 stored byte for byte and at most
/// `max_file_bytes`; the bound is at least one byte per file byte, so every
/// stored value is readable within it — a `substr` of its first character works — and the bound
/// itself is still enforced on computed values (proved by the zeroblob
/// test above).
#[test]
fn a_stored_value_is_readable_within_the_length_bound_and_substr_works() -> TestResult {
    let file_bytes = 64 * 1024;
    let body = "a".repeat(file_bytes);
    let project = activated_project("fits.txt", body.as_bytes())?;
    let sql = "SELECT substr(content, 1, 1) AS head, length(CAST(content AS BLOB)) AS n \
               FROM files WHERE path = 'fits.txt'";
    let (at, bound) = ask_bounded(&project, sql, file_bytes)?;

    ensure_eq(
        &usize::try_from(bound).check("bound")?,
        &file_bytes,
        "usize::try_from(bound).check(\"bound\")? == file_bytes",
    )?;
    let expected = serde_json::json!({"head": "a", "n": file_bytes});
    ensure_eq(
        &at.check("the stored value reads")?.rows,
        &vec![expected],
        "at.check(\"the stored value reads\")?.rows == vec![expected]",
    )?;
    Ok(())
}

/// The value bound is the larger of the query value cap and one byte per
/// file byte, clamped to SQLite's maximum; the SQL bound is the tool's SQL
/// cap plus the wrapper overhead.
#[test]
fn engine_limits_cover_stored_values_and_the_wrapped_sql() -> TestResult {
    use super::exec::{EngineLimits, engine_limits};
    let with = |file: u64, value: usize, sql: usize| ResourceLimits {
        max_file_bytes: file,
        max_query_value_bytes: value,
        max_query_sql_bytes: sql,
        ..ResourceLimits::default()
    };

    for (limits, value_length, sql_length) in [
        (with(1000, 1000, 100), 1000, 164),
        (with(1000, 10, 100), 1000, 164),
        (with(1000, 5000, 100), 5000, 164),
        (
            with(1_000_000_000, 1, 2_000_000_000),
            1_000_000_000,
            1_000_000_000,
        ),
    ] {
        let expected = EngineLimits {
            value_length,
            sql_length,
        };
        ensure_eq(
            &engine_limits(&limits).check("computable")?,
            &expected,
            format!("{limits:?}"),
        )?;
    }
    Ok(())
}
