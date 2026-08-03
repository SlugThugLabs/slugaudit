use super::*;
use crate::model::EvidenceLimits;

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

fn total_payload_bytes(items: &[EvidenceItem]) -> usize {
    items
        .iter()
        .filter(|item| item.key != "evidence:truncated")
        .map(|item| {
            serde_json::to_vec(&item.payload)
                .expect("encode payload")
                .len()
        })
        .sum()
}

/// Reproduces a real production failure: 82 items each individually well
/// under the 64 KiB per-item cap, and 82 items is nowhere near the 10,000
/// item-count cap, yet their combined serialized payload was 5,326,990
/// bytes — over the 4 MiB (4,194,304-byte) per-file cap. The old
/// truncate-then-retain implementation checked count and per-item size
/// only, so it let the whole oversized set through.
#[test]
fn reproduces_the_5_326_990_byte_cumulative_overflow() {
    let limits = ResourceLimits::default();
    let mut items: Vec<EvidenceItem> = (0..81)
        .map(|i| item_with_content_bytes(i, 65_000))
        .collect();
    items.push(item_with_content_bytes(81, 61_826));
    assert_eq!(
        items
            .iter()
            .map(|item| serde_json::to_vec(&item.payload).unwrap().len())
            .sum::<usize>(),
        5_326_990,
        "fixture must reproduce the exact reported overflow size"
    );

    apply_evidence_limits(&mut items, &limits);

    assert!(
        total_payload_bytes(&items) <= limits.evidence.max_payload_bytes_per_file,
        "cumulative payload must never exceed the per-file cap"
    );
    assert!(
        has_truncation_marker(&items),
        "truncation must be recorded as evidence"
    );
}

#[test]
fn evidence_within_every_cap_is_untouched() {
    let limits = ResourceLimits::default();
    let mut items: Vec<EvidenceItem> = (0..5).map(|i| item_with_content_bytes(i, 100)).collect();
    let original = items.clone();

    apply_evidence_limits(&mut items, &limits);

    assert_eq!(
        items, original,
        "nothing should change when every cap is satisfied"
    );
    assert!(!has_truncation_marker(&items));
}

#[test]
fn an_oversized_single_item_is_dropped_and_recorded() {
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

    apply_evidence_limits(&mut items, &limits);

    assert_eq!(items.len(), 2, "the kept item plus the truncation marker");
    assert!(items.iter().any(|item| item.key == "structure:0"));
    assert!(has_truncation_marker(&items));
}

#[test]
fn item_count_cap_truncates_and_reserves_room_for_the_marker() {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_items_per_file: 3,
            max_payload_bytes_per_item: 1000,
            max_payload_bytes_per_file: 100_000,
        },
        ..ResourceLimits::default()
    };
    let mut items: Vec<EvidenceItem> = (0..10).map(|i| item_with_content_bytes(i, 10)).collect();

    apply_evidence_limits(&mut items, &limits);

    // Cap is 3; one slot is reserved for the marker itself, so at most 2
    // real items survive plus the marker — never more than the cap total.
    assert!(items.len() <= 3);
    assert!(has_truncation_marker(&items));
}

#[test]
fn cumulative_byte_cap_stops_before_the_file_cap_not_after() {
    let limits = ResourceLimits {
        evidence: EvidenceLimits {
            max_items_per_file: 1000,
            max_payload_bytes_per_item: 1000,
            max_payload_bytes_per_file: 2000,
        },
        ..ResourceLimits::default()
    };
    let mut items: Vec<EvidenceItem> = (0..10).map(|i| item_with_content_bytes(i, 300)).collect();

    apply_evidence_limits(&mut items, &limits);

    assert!(total_payload_bytes(&items) <= limits.evidence.max_payload_bytes_per_file);
    assert!(items.len() < 10, "later items must have been dropped");
    assert!(has_truncation_marker(&items));
}
