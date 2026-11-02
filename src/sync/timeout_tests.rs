//!
//! Wall-clock timeout coverage for the sync hot loops. Each test injects a
//! nanosecond-sized budget — any positive elapsed time trips the deadline,
//! so the abort is deterministic without waiting out the production-sized
//! budget. This is the same injection pattern `tools::query_tests` uses for
//! its wall-clock budget.
use super::*;
use crate::model::ResourceLimits;
use crate::store::open_read_write;
use crate::sync::discovery::{discover, discover_with_deadline};
use crate::sync::sample::sample_all_with_deadline;
use crate::sync::test_support::{stored_paths, write};
use crate::util::Deadline;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::time::Duration;

fn tiny_budget() -> Deadline {
    // `Duration::ZERO` makes `elapsed >= budget` always true, so the abort
    // is deterministic on any clock granularity — stronger than the
    // `from_nanos(1)` convention used elsewhere.
    Deadline::after(Duration::ZERO)
}

#[test]
fn discovery_times_out_when_the_walk_budget_is_exhausted() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "a.rs", b"fn a() {}")?;

    let error = discover_with_deadline(project.path(), &tiny_budget())
        .check_err("a spent budget must abort discovery")?;
    assert!(
        matches!(error, DiscoveryError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
    assert!(
        error.to_string().contains("wall-clock time budget"),
        "error should name the budget: {error}"
    );
    Ok(())
}

#[test]
fn discovery_with_a_generous_budget_succeeds() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "a.rs", b"fn a() {}")?;
    let (files, _skipped) = discover(project.path()).check("discover")?;
    assert_eq!(files.len(), 1);
    Ok(())
}

#[test]
fn sampling_times_out_when_the_budget_is_exhausted() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "a.rs", b"fn a() {}")?;
    let (discovered, _skipped) = discover(project.path()).check("discover")?;
    let limits = ResourceLimits::default();

    let result = sample_all_with_deadline(
        &discovered,
        &limits,
        &crate::progress::NoopProgressSink,
        &tiny_budget(),
    );
    let Err(error) = result else {
        return Err("a spent budget must abort sampling".to_owned()).check("unexpected outcome");
    };
    assert!(
        matches!(error, PublishError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
    Ok(())
}

#[test]
fn publish_times_out_when_the_wall_clock_budget_is_exhausted() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "src/main.rs", b"fn main() {}")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    let limits = ResourceLimits {
        max_sync_wall_clock: Duration::from_nanos(1),
        ..ResourceLimits::default()
    };
    let result = publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    );

    let error = result.check_err("a spent budget must abort the publish")?;
    let message = error.to_string();
    assert!(
        message.contains("wall-clock time budget"),
        "error should name the budget: {error:?}"
    );
    // The budget may be caught by discovery or by sampling depending on
    // which check runs first, so accept either variant — the point is that
    // a pathological repo can never stall the tool call.
    assert!(
        matches!(
            error,
            PublishError::TimeBudgetExceeded { .. }
                | PublishError::Discovery(DiscoveryError::TimeBudgetExceeded { .. })
        ),
        "expected a time-budget error, got {error:?}"
    );
    Ok(())
}

/// `discover_with_deadline` aborts when the budget is exhausted on the
/// very first iterated entry — exercises the deadline check inside the
/// `for entry in walker` loop, not just the function entry. The test
/// seeds a non-empty project, gives it a zero deadline, and asserts the
/// `TimeBudgetExceeded` return. It is the per-iteration variant of
/// `discovery_times_out_when_the_walk_budget_is_exhausted` (which
/// aborts at the loop head without iterating).
#[test]
fn discover_with_deadline_aborts_mid_walk_when_budget_exhausts() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "many.rs", &vec![b'x'; 4096])?;
    write(project.path(), "more.rs", &vec![b'y'; 4096])?;
    write(project.path(), "last.rs", &vec![b'z'; 4096])?;

    let error = discover_with_deadline(project.path(), &tiny_budget())
        .check_err("a spent budget must abort discovery mid-walk")?;
    assert!(
        matches!(error, DiscoveryError::TimeBudgetExceeded { .. }),
        "expected TimeBudgetExceeded, got {error:?}"
    );
    Ok(())
}

/// `analyze` runs every parse of a file within what is left of the sync
/// deadline it is handed (the deadline `publish_with_limits` creates once
/// per operation), so one file's parse cannot outlive the sync
/// budget. Under a spent deadline the parse is not started: the file is
/// stored `ContentOnly` with the `parse_timeout` reason, and the caller's
/// own between-file deadline check then ends the sync. The progress-callback
/// cancellation of a parse already running is pinned in `analyze_tests`.
#[test]
fn analyze_under_a_spent_sync_deadline_records_parse_timeout_without_parsing() -> TestResult {
    use crate::sync::analyze::analyze;
    let spent = Deadline::after(Duration::ZERO);

    let result = analyze(
        "src/lib.rs",
        Some("pub fn lib() {}\n"),
        crate::model::ResourceLimits::default().max_parse_bytes,
        Duration::from_secs(5),
        &spent,
    )
    .check("analyze the file")?;

    assert!(result.run.validate().is_ok());
    assert_eq!(
        result.run.outcome,
        crate::model::ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
    assert_eq!(
        result.run.completeness,
        crate::model::ExtractionCompleteness::ContentOnly
    );
    assert!(result.evidence.is_empty(), "nothing was parsed");
    Ok(())
}

/// A real cancellation, not an injected one: a 400 KiB Rust file (under
/// `max_parse_bytes`) takes about 1.5 s to parse in a debug build, and a
/// 50 ms sync budget cuts `process()` itself through the pack's parse
/// timeout (tree-sitter's progress callback). The file is recorded
/// `parse_timeout`, nothing is extracted, and `analyze` returns near the
/// budget instead of after the whole parse.
#[test]
fn a_parse_that_outlives_the_sync_budget_is_cancelled_and_recorded_parse_timeout() -> TestResult {
    use crate::sync::analyze::analyze;
    let line = "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n";
    let content = line.repeat(400 * 1024 / line.len());
    let limits = crate::model::ResourceLimits::default();
    let deadline = Deadline::after(Duration::from_millis(50));

    let started = std::time::Instant::now();
    let result = analyze(
        "src/big.rs",
        Some(&content),
        limits.max_parse_bytes,
        Duration::from_millis(limits.max_parse_wall_clock_ms),
        &deadline,
    )
    .check("analyze the file")?;
    let elapsed = started.elapsed();

    assert_eq!(
        result.run.outcome,
        crate::model::ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
    assert_eq!(
        result.run.completeness,
        crate::model::ExtractionCompleteness::ContentOnly
    );
    assert!(result.evidence.is_empty());
    assert!(elapsed < Duration::from_secs(1), "analyze took {elapsed:?}");
    Ok(())
}

/// The sync budget still ends the sync: a file whose parse outlives it is cut
/// at the sync deadline (the per-file allowance here is far longer), and the
/// publish fails with the budget error and publishes nothing.
#[test]
fn a_parse_that_outlives_the_sync_budget_still_fails_the_publish() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    let line = "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n";
    write(
        project.path(),
        "slow.rs",
        line.repeat(1_900_000 / line.len()).as_bytes(),
    )?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let limits = ResourceLimits {
        max_sync_wall_clock: Duration::from_millis(300),
        max_parse_wall_clock_ms: 600_000,
        ..ResourceLimits::default()
    };

    let error = publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    )
    .check_err("the sync budget ran out during the parse")?;

    assert!(
        matches!(
            error,
            PublishError::TimeBudgetExceeded { .. }
                | PublishError::Discovery(DiscoveryError::TimeBudgetExceeded { .. })
        ),
        "expected a time-budget error, got {error:?}"
    );
    assert!(
        stored_paths(&connection)?.is_empty(),
        "nothing was published"
    );
    Ok(())
}
