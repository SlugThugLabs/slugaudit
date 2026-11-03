//! The size and time bounds on `analyze`: content over
//! `max_parse_bytes` never reaches the pack, and a parse cut at the sync
//! deadline is recorded with the `parse_timeout` reason. The real
//! (not injected) cancellations are pinned in `sync::timeout_tests`.

use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};
use crate::sync::analyze_seam::forced_failure;
use crate::util::BudgetBound;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::time::Duration;

/// A per-file allowance no test parse comes near.
const PER_FILE: Duration = Duration::from_mins(10);

/// A deadline no test parse comes near.
fn generous_deadline() -> Deadline {
    Deadline::after(Duration::from_mins(10))
}

/// `analyze` under the default `max_parse_bytes`.
fn analyze_default(relative_path: &str, content: &str) -> TestResult<ParseResult> {
    analyze(
        relative_path,
        Some(content),
        2 * 1024 * 1024,
        PER_FILE,
        &generous_deadline(),
    )
    .check("analyze the file")
}

#[test]
fn content_over_max_parse_bytes_is_content_only_without_reaching_the_loader() -> TestResult {
    let before = forced_failure::calls();

    let result = analyze(
        "big.rs",
        Some("fn a() {}\n"),
        4,
        PER_FILE,
        &generous_deadline(),
    )
    .check("analyze the file")?;

    ensure_eq(
        &forced_failure::calls(),
        &before,
        "the pack is never called",
    )?;
    ensure_eq(
        &result.language.as_deref(),
        &Some("rust"),
        "result.language.as_deref() == Some(\"rust\")",
    )?;
    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Unavailable,
        "result.run.availability == ParserAvailability::Unavailable",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "result.run.completeness == ExtractionCompleteness::ContentOnly",
    )?;
    ensure_eq(
        &result.run.error_reason().as_deref(),
        &Some("oversize_for_parse"),
        "result.run.error_reason().as_deref() == Some(\"oversize_for_parse\")",
    )?;
    ensure(result.evidence.is_empty(), "result.evidence.is_empty()")?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn content_at_exactly_max_parse_bytes_is_parsed() -> TestResult {
    let content = "fn a() {}\n";

    let result = analyze(
        "edge.rs",
        Some(content),
        content.len() as u64,
        PER_FILE,
        &generous_deadline(),
    )
    .check("analyze the file")?;

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Succeeded,
        "result.run.outcome == ParseOutcome::Succeeded",
    )?;
    Ok(())
}

#[test]
fn a_parse_cancelled_by_the_pack_is_recorded_as_parse_timeout() -> TestResult {
    let _forced = forced_failure::force(|| PackError::ParseTimeout { timeout_ms: 1 });

    let result = analyze_default("slow.rs", "fn a() {}")?;

    ensure_eq(
        &result.run.availability,
        &ParserAvailability::Available,
        "result.run.availability == ParserAvailability::Available",
    )?;
    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: "parse_timeout".to_owned(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: \"parse_timeout\".to_owned() }",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "result.run.completeness == ExtractionCompleteness::ContentOnly",
    )?;
    Ok(())
}

#[test]
fn a_re_parse_cut_at_the_deadline_keeps_the_pack_evidence_as_partial_parse_timeout() -> TestResult {
    let pack_items = analyze_default("lib.rs", "pub fn a() {}\n")?.evidence;
    let kept = pack_items.len();
    ensure(kept > 0, "the fixture extracts evidence")?;

    let result = timeout_result(
        "lib.rs",
        "rust".to_owned(),
        pack_items,
        BudgetBound::Operation,
    );

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: "parse_timeout".to_owned(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: \"parse_timeout\".to_owned() }",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::Partial,
        "result.run.completeness == ExtractionCompleteness::Partial",
    )?;
    ensure_eq(
        &result.evidence.len(),
        &kept,
        "result.evidence.len() == kept",
    )?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn a_spent_per_file_allowance_records_parse_timeout_without_parsing() -> TestResult {
    let result = analyze(
        "lib.rs",
        Some("pub fn a() {}\n"),
        2 * 1024 * 1024,
        Duration::ZERO,
        &generous_deadline(),
    )
    .check("analyze the file")?;

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: "parse_timeout".to_owned(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: \"parse_timeout\".to_owned() }",
    )?;
    ensure_eq(
        &result.run.completeness,
        &ExtractionCompleteness::ContentOnly,
        "result.run.completeness == ExtractionCompleteness::ContentOnly",
    )?;
    ensure(result.evidence.is_empty(), "result.evidence.is_empty()")?;
    result.run.validate().check("the parser run validates")?;
    Ok(())
}

#[test]
fn a_per_file_allowance_longer_than_the_sync_budget_yields_to_the_sync_budget() -> TestResult {
    let spent_sync = Deadline::after(Duration::ZERO);

    let result = analyze(
        "lib.rs",
        Some("pub fn a() {}\n"),
        2 * 1024 * 1024,
        PER_FILE,
        &spent_sync,
    )
    .check("analyze the file")?;

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: "parse_timeout".to_owned(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: \"parse_timeout\".to_owned() }",
    )?;
    Ok(())
}

#[test]
fn a_one_millisecond_per_file_allowance_cuts_a_large_parse_and_leaves_the_sync_budget() -> TestResult
{
    let line = "pub fn f(x: Option<u8>) -> u8 { x.expect() }\n";
    let content = line.repeat(400 * 1024 / line.len());
    let sync = generous_deadline();

    let result = analyze(
        "big.rs",
        Some(&content),
        2 * 1024 * 1024,
        Duration::from_millis(1),
        &sync,
    )
    .check("analyze the file")?;

    ensure_eq(
        &result.run.outcome,
        &ParseOutcome::Failed {
            reason: "parse_timeout".to_owned(),
        },
        "result.run.outcome == ParseOutcome::Failed { reason: \"parse_timeout\".to_owned() }",
    )?;
    ensure(sync.exceeded().is_none(), "the sync budget is untouched")?;
    Ok(())
}
