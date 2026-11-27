//! Evidence caps: what `apply_evidence_limits` discards, the marker it leaves,
//! and the completeness downgrade `to_file_record` applies.
use super::*;
use crate::model::{EvidenceLimits, ExportPayload, ExtractionCompleteness};
use crate::sync::test_support::generous_deadline;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

const EXPORT_WRAPPER_BYTES: usize = 26;

fn item_with_content_bytes(index: usize, content_len: usize) -> EvidenceItem {
    // `ExportPayload { name, kind: "Named" }` serializes as
    // `{"name":"...","kind":"Named"}`, whose wrapper is 26 bytes plus `name.len()`.
    EvidenceItem {
        key: format!("export:{index}"),
        kind: EvidenceKind::Export,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: EvidencePayload::Export(ExportPayload {
            name: "a".repeat(content_len),
            kind: "Named".to_owned(),
        }),
    }
}

fn has_truncation_marker(items: &[EvidenceItem]) -> bool {
    items.iter().any(|item| item.key == "evidence:truncated")
}

fn total_payload_bytes(items: &[EvidenceItem]) -> TestResult<usize> {
    let mut total = 0;
    for item in items.iter().filter(|item| item.key != "evidence:truncated") {
        let payload_len = util::serialized_len(&item.payload).check("encode payload")?;
        total = usize::checked_add(total, payload_len).check("payload total fits")?;
    }
    Ok(total)
}

#[test]
fn reproduces_the_5_326_990_byte_cumulative_overflow() -> TestResult {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_payload_bytes_per_file: 4 * 1024 * 1024,
            ..EvidenceLimits::default()
        },
        ..ResourceLimits::default()
    };
    let mut items: Vec<EvidenceItem> = (0..81)
        .map(|i| item_with_content_bytes(i, 65_002 - EXPORT_WRAPPER_BYTES))
        .collect();
    items.push(item_with_content_bytes(81, 61_828 - EXPORT_WRAPPER_BYTES));
    let mut serialized = 0usize;
    for item in &items {
        serialized = serialized
            .checked_add(util::serialized_len(&item.payload).check("payload serializes")?)
            .check("serialized sum fits")?;
    }
    ensure_eq(
        &serialized,
        &5_326_990,
        "fixture must reproduce the exact reported overflow size",
    )?;

    ensure(
        apply_evidence_limits(&mut items, &limits),
        "apply_evidence_limits(&mut items, &limits)",
    )?;

    ensure(
        total_payload_bytes(&items)? <= limits.evidence.max_payload_bytes_per_file,
        "cumulative payload must never exceed the per-file cap",
    )?;
    ensure(
        has_truncation_marker(&items),
        "truncation must be recorded as evidence",
    )?;
    Ok(())
}

#[test]
fn evidence_within_every_cap_is_untouched() -> TestResult {
    let limits = ResourceLimits::default();
    let mut items: Vec<EvidenceItem> = (0..5).map(|i| item_with_content_bytes(i, 100)).collect();
    let original = items.clone();

    ensure(
        !apply_evidence_limits(&mut items, &limits),
        "!apply_evidence_limits(&mut items, &limits)",
    )?;

    ensure_eq(
        &items,
        &original,
        "nothing should change when every cap is satisfied",
    )?;
    ensure(
        !has_truncation_marker(&items),
        "!has_truncation_marker(&items)",
    )?;
    Ok(())
}

#[test]
fn an_oversized_single_item_is_dropped_and_recorded() -> TestResult {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_items_per_file: 100,
            max_payload_bytes_per_item: 50,
            max_payload_bytes_per_file: 10_000,
        },
        ..ResourceLimits::default()
    };
    let mut items = vec![
        item_with_content_bytes(0, 10),
        item_with_content_bytes(1, 500),
    ];

    ensure(
        apply_evidence_limits(&mut items, &limits),
        "apply_evidence_limits(&mut items, &limits)",
    )?;

    ensure_eq(&items.len(), &2, "the kept item plus the truncation marker")?;
    ensure(
        items.iter().any(|item| item.key == "export:0"),
        "items.iter().any(|item| item.key == \"export:0\")",
    )?;
    ensure(
        has_truncation_marker(&items),
        "has_truncation_marker(&items)",
    )?;
    Ok(())
}

#[test]
fn item_count_cap_truncates_and_reserves_room_for_the_marker() -> TestResult {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_items_per_file: 3,
            max_payload_bytes_per_item: 1000,
            max_payload_bytes_per_file: 100_000,
        },
        ..ResourceLimits::default()
    };
    let mut items: Vec<EvidenceItem> = (0..10).map(|i| item_with_content_bytes(i, 10)).collect();

    ensure(
        apply_evidence_limits(&mut items, &limits),
        "apply_evidence_limits(&mut items, &limits)",
    )?;

    ensure(items.len() <= 3, "items.len() <= 3")?;
    ensure(
        has_truncation_marker(&items),
        "has_truncation_marker(&items)",
    )?;
    Ok(())
}

#[test]
fn cumulative_byte_cap_stops_before_the_file_cap_not_after() -> TestResult {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_items_per_file: 1000,
            max_payload_bytes_per_item: 1000,
            max_payload_bytes_per_file: 2000,
        },
        ..ResourceLimits::default()
    };
    let mut items: Vec<EvidenceItem> = (0..10).map(|i| item_with_content_bytes(i, 300)).collect();

    ensure(
        apply_evidence_limits(&mut items, &limits),
        "apply_evidence_limits(&mut items, &limits)",
    )?;

    ensure(
        total_payload_bytes(&items)? <= limits.evidence.max_payload_bytes_per_file,
        "total_payload_bytes(&items)? <= limits.evidence.max_payload_bytes_per_file",
    )?;
    ensure(items.len() < 10, "later items must have been dropped")?;
    ensure(
        has_truncation_marker(&items),
        "has_truncation_marker(&items)",
    )?;
    Ok(())
}

#[test]
fn evidence_limit_truncation_downgrades_a_full_run_to_partial() -> TestResult {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_items_per_file: 1,
            ..EvidenceLimits::default()
        },
        ..ResourceLimits::default()
    };
    let content = "pub fn a() {}\npub fn b() {}\n";
    let sample = Sample {
        relative_path: "lib.rs".to_owned(),
        is_binary: false,
        content: Some(content.to_owned()),
        identity: hash::hash_bytes("lib.rs", content.as_bytes()),
        byte_len: content.len() as u64,
        modified_unix_seconds: None,
    };

    let record =
        to_file_record(sample, &limits, &generous_deadline()).check("build the file record")?;

    ensure(
        has_truncation_marker(&record.evidence),
        "has_truncation_marker(&record.evidence)",
    )?;
    ensure_eq(
        &record.run.completeness,
        &ExtractionCompleteness::Partial,
        "record.run.completeness == ExtractionCompleteness::Partial",
    )?;
    Ok(())
}

#[test]
fn a_full_run_within_every_evidence_cap_stays_full() -> TestResult {
    let content = "pub fn a() {}\npub fn b() {}\n";
    let sample = Sample {
        relative_path: "lib.rs".to_owned(),
        is_binary: false,
        content: Some(content.to_owned()),
        identity: hash::hash_bytes("lib.rs", content.as_bytes()),
        byte_len: content.len() as u64,
        modified_unix_seconds: None,
    };

    let record = to_file_record(sample, &ResourceLimits::default(), &generous_deadline())
        .check("build the file record")?;

    ensure(
        !has_truncation_marker(&record.evidence),
        "!has_truncation_marker(&record.evidence)",
    )?;
    ensure_eq(
        &record.run.completeness,
        &ExtractionCompleteness::Full,
        "record.run.completeness == ExtractionCompleteness::Full",
    )?;
    Ok(())
}

#[test]
fn the_truncation_marker_names_items_that_failed_to_serialize_only_when_there_were_some()
-> TestResult {
    let without = truncation_evidence(5, 3, 1, 0, 1);
    let with = truncation_evidence(5, 2, 1, 2, 0);

    let message = |item: &EvidenceItem| -> TestResult<String> {
        match &item.payload {
            EvidencePayload::Diagnostic(diag) => Ok(diag.message.clone()),
            other => Err(format!("expected Diagnostic payload, got {other:?}")).check("payload"),
        }
    };
    ensure(
        !message(&without)?.contains("failed to serialize"),
        "!message(&without)?.contains(\"failed to serialize\")",
    )?;
    ensure(
        message(&with)?.contains("2 item(s) failed to serialize"),
        &message(&with)?,
    )?;
    ensure(
        message(&with)?.contains("0 item(s) dropped by the item-count"),
        format!(
            "unmeasurable items must not be double-counted as cap drops: {}",
            message(&with)?
        ),
    )?;
    Ok(())
}
