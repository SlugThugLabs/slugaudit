//! Evidence caps: what `apply_evidence_limits` discards, the marker it leaves,
//! and the completeness downgrade `to_file_record` applies.
use super::*;
use crate::model::{EvidenceLimits, ExtractionCompleteness};
use slugaudit_test_check::{Check, TestResult};

fn item_with_content_bytes(index: usize, content_len: usize) -> EvidenceItem {
    // A bare JSON string payload serializes as `"` + content + `"`, so its
    // byte length is exactly `content_len + 2` — no escaping, no object
    // overhead to account for, which keeps the arithmetic in these tests
    // exact instead of approximate.
    EvidenceItem {
        key: format!("structure:{index}"),
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: serde_json::Value::String("a".repeat(content_len)),
    }
}

fn has_truncation_marker(items: &[EvidenceItem]) -> bool {
    items.iter().any(|item| item.key == "evidence:truncated")
}

fn total_payload_bytes(items: &[EvidenceItem]) -> TestResult<usize> {
    let mut total = 0;
    for item in items.iter().filter(|item| item.key != "evidence:truncated") {
        total += serde_json::to_vec(&item.payload)
            .check("encode payload")?
            .len();
    }
    Ok(total)
}

/// 82 items each individually well under the 64 KiB per-item cap, and 82
/// items is nowhere near the 10,000 item-count cap, yet their combined
/// serialized payload is 5,326,990 bytes — over the 4 MiB (4,194,304-byte)
/// per-file cap. A check of count and per-item size only would let the whole
/// oversized set through, so the cumulative cap must catch it.
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
        .map(|i| item_with_content_bytes(i, 65_000))
        .collect();
    items.push(item_with_content_bytes(81, 61_826));
    let mut serialized = 0;
    for item in &items {
        serialized += serde_json::to_vec(&item.payload)
            .check("payload serializes")?
            .len();
    }
    assert_eq!(
        serialized, 5_326_990,
        "fixture must reproduce the exact reported overflow size"
    );

    assert!(apply_evidence_limits(&mut items, &limits));

    assert!(
        total_payload_bytes(&items)? <= limits.evidence.max_payload_bytes_per_file,
        "cumulative payload must never exceed the per-file cap"
    );
    assert!(
        has_truncation_marker(&items),
        "truncation must be recorded as evidence"
    );
    Ok(())
}

#[test]
fn evidence_within_every_cap_is_untouched() -> TestResult {
    let limits = ResourceLimits::default();
    let mut items: Vec<EvidenceItem> = (0..5).map(|i| item_with_content_bytes(i, 100)).collect();
    let original = items.clone();

    assert!(!apply_evidence_limits(&mut items, &limits));

    assert_eq!(
        items, original,
        "nothing should change when every cap is satisfied"
    );
    assert!(!has_truncation_marker(&items));
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

    assert!(apply_evidence_limits(&mut items, &limits));

    assert_eq!(items.len(), 2, "the kept item plus the truncation marker");
    assert!(items.iter().any(|item| item.key == "structure:0"));
    assert!(has_truncation_marker(&items));
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

    assert!(apply_evidence_limits(&mut items, &limits));

    // Cap is 3; one slot is reserved for the marker itself, so at most 2
    // real items survive plus the marker — never more than the cap total.
    assert!(items.len() <= 3);
    assert!(has_truncation_marker(&items));
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

    assert!(apply_evidence_limits(&mut items, &limits));

    assert!(total_payload_bytes(&items)? <= limits.evidence.max_payload_bytes_per_file);
    assert!(items.len() < 10, "later items must have been dropped");
    assert!(has_truncation_marker(&items));
    Ok(())
}

/// A file that parsed cleanly is `Full` — the completeness the evidence caps
/// can overclaim. Once the caps discard any item, the stored run must
/// downgrade to `Partial` rather than assert `Full` beside its own truncation
/// marker.
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

    assert!(has_truncation_marker(&record.evidence));
    assert_eq!(record.run.completeness, ExtractionCompleteness::Partial);
    Ok(())
}

/// The downgrade must be conditional: a clean parse with room for every item
/// stays `Full`, so the downgrade cannot simply erase `Full` everywhere.
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

    assert!(!has_truncation_marker(&record.evidence));
    assert_eq!(record.run.completeness, ExtractionCompleteness::Full);
    Ok(())
}

#[test]
fn the_truncation_marker_names_items_that_failed_to_serialize_only_when_there_were_some()
-> TestResult {
    let without = truncation_evidence(5, 3, 1, 0);
    let with = truncation_evidence(5, 2, 1, 2);

    let message = |item: &EvidenceItem| -> TestResult<String> {
        Ok(item.payload["message"]
            .as_str()
            .check("the truncation marker has a text message")?
            .to_owned())
    };
    assert!(!message(&without)?.contains("failed to serialize"));
    assert!(
        message(&with)?.contains("2 item(s) failed to serialize"),
        "{}",
        message(&with)?
    );
    assert!(
        message(&with)?.contains("0 item(s) dropped by the item-count"),
        "unmeasurable items must not be double-counted as cap drops: {}",
        message(&with)?
    );
    Ok(())
}
