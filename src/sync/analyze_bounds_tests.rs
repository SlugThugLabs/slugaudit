//! The size and time bounds on `analyze` (A4): content over
//! `max_parse_bytes` never reaches the pack, and a parse cut at the sync
//! deadline is recorded with the `parse_timeout` reason. The real
//! (not injected) cancellations are pinned in `sync::timeout_tests`.

use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};
use crate::sync::analyze_seam::forced_failure;
use crate::util::BudgetBound;
use std::time::Duration;

/// A per-file allowance no test parse comes near.
const PER_FILE: Duration = Duration::from_secs(600);

/// A deadline no test parse comes near.
fn generous_deadline() -> Deadline {
    Deadline::after(Duration::from_secs(600))
}

/// `analyze` under the default `max_parse_bytes`.
fn analyze_default(relative_path: &str, content: &str) -> ParseResult {
    analyze(
        relative_path,
        Some(content),
        2 * 1024 * 1024,
        PER_FILE,
        &generous_deadline(),
    )
}

#[test]
fn content_over_max_parse_bytes_is_content_only_without_reaching_the_loader() {
    let before = forced_failure::calls();

    let result = analyze(
        "big.rs",
        Some("fn a() {}\n"),
        4,
        PER_FILE,
        &generous_deadline(),
    );

    assert_eq!(forced_failure::calls(), before, "the pack is never called");
    assert_eq!(result.language.as_deref(), Some("rust"));
    assert_eq!(result.run.availability, ParserAvailability::Unavailable);
    assert_eq!(result.run.completeness, ExtractionCompleteness::ContentOnly);
    assert_eq!(
        result.run.error_reason().as_deref(),
        Some("oversize_for_parse")
    );
    assert!(result.evidence.is_empty());
    assert!(result.run.validate().is_ok());
}

#[test]
fn content_at_exactly_max_parse_bytes_is_parsed() {
    let content = "fn a() {}\n";

    let result = analyze(
        "edge.rs",
        Some(content),
        content.len() as u64,
        PER_FILE,
        &generous_deadline(),
    );

    assert_eq!(result.run.outcome, ParseOutcome::Succeeded);
}

#[test]
fn a_parse_cancelled_by_the_pack_is_recorded_as_parse_timeout() {
    let _forced = forced_failure::force(|| PackError::ParseTimeout { timeout_ms: 1 });

    let result = analyze_default("slow.rs", "fn a() {}");

    assert_eq!(result.run.availability, ParserAvailability::Available);
    assert_eq!(
        result.run.outcome,
        ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
    assert_eq!(result.run.completeness, ExtractionCompleteness::ContentOnly);
}

#[test]
fn a_re_parse_cut_at_the_deadline_keeps_the_pack_evidence_as_partial_parse_timeout() {
    let pack_items = analyze_default("lib.rs", "pub fn a() {}\n").evidence;
    let kept = pack_items.len();
    assert!(kept > 0, "the fixture extracts evidence");

    let result = timeout_result(
        "lib.rs",
        "rust".to_owned(),
        pack_items,
        BudgetBound::Operation,
    );

    assert_eq!(
        result.run.outcome,
        ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
    assert_eq!(result.run.completeness, ExtractionCompleteness::Partial);
    assert_eq!(result.evidence.len(), kept);
    assert!(result.run.validate().is_ok());
}

#[test]
fn a_spent_per_file_allowance_records_parse_timeout_without_parsing() {
    let result = analyze(
        "lib.rs",
        Some("pub fn a() {}\n"),
        2 * 1024 * 1024,
        Duration::ZERO,
        &generous_deadline(),
    );

    assert_eq!(
        result.run.outcome,
        ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
    assert_eq!(result.run.completeness, ExtractionCompleteness::ContentOnly);
    assert!(result.evidence.is_empty());
    assert!(result.run.validate().is_ok());
}

#[test]
fn a_per_file_allowance_longer_than_the_sync_budget_yields_to_the_sync_budget() {
    let spent_sync = Deadline::after(Duration::ZERO);

    let result = analyze(
        "lib.rs",
        Some("pub fn a() {}\n"),
        2 * 1024 * 1024,
        PER_FILE,
        &spent_sync,
    );

    assert_eq!(
        result.run.outcome,
        ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
}

#[test]
fn a_one_millisecond_per_file_allowance_cuts_a_large_parse_and_leaves_the_sync_budget() {
    let line = "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n";
    let content = line.repeat(400 * 1024 / line.len());
    let sync = generous_deadline();

    let result = analyze(
        "big.rs",
        Some(&content),
        2 * 1024 * 1024,
        Duration::from_millis(1),
        &sync,
    );

    assert_eq!(
        result.run.outcome,
        ParseOutcome::Failed {
            reason: "parse_timeout".to_owned()
        }
    );
    assert!(sync.exceeded().is_none(), "the sync budget is untouched");
}
