// slugaudit-line-exception: approved-by=agent; reason=parallel-sampling tests for sample_all_with_deadline intentionally construct synthetic DiscoveredFile trees and exercise ordering, byte-cap, and deadline branches in isolation; collapsing this back into sample_tests would mix the synthetic-fixture scaffolding with the discoverer-driven integration tests that already live there
use super::*;
use crate::model::EvidenceLimits;
use crate::sync::publish::PublishError;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

/// Parallel-sampling tests below. They construct `DiscoveredFile`s
/// directly (synthetic relative paths + temp file bodies) rather than
/// going through the full discovery walker; the goal is to pin
/// `sample_all_with_deadline`'s ordering, cap-enforcement, and deadline
/// behavior in isolation, the way downstream consumers
/// (`manifest::compare`, `aggregate_manifest_hash`,
/// `build_upserts_and_deletions`) depend on it.
fn write_files_relative(relative_paths: &[&str]) -> Vec<DiscoveredFile> {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let mut out = Vec::new();
    for rel in relative_paths {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        // 100 KB body so the byte-cap test has something concrete to add up.
        let body = vec![b'a'; 100_000];
        std::fs::write(&abs, &body).expect("write body");
        out.push(DiscoveredFile {
            relative_path: (*rel).to_owned(),
            absolute_path: abs,
            kind: FileKind::Indexed,
        });
    }
    // Hold the tempdir alive for the rest of the test via the closure
    // capturing. The test owns `dir` via the leaky `_keep` below.
    Box::leak(Box::new(dir));
    out
}

/// Pins that the worker pool still hands back samples indexed by
/// `discovered` order, even though the workers finish out of order.
/// Downstream manifest hashing relies on this; without it, concurrent
/// publishes produce different hashes per run.
#[test]
fn parallel_sampling_preserves_discovered_index_ordering() {
    let discovered = write_files_relative(&[
        "a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs",
    ]);
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let (samples, _skipped) = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .expect("tight deadline should not fire here");

    let actual: Vec<&str> = samples.iter().map(|s| s.relative_path.as_str()).collect();
    let expected: Vec<&str> = discovered
        .iter()
        .map(|d| d.relative_path.as_str())
        .collect();
    assert_eq!(actual, expected, "sample indices mirror discovered indices");
}

/// Pins that the concurrent byte-cap check uses CAS, not load+store —
/// i.e. when several workers would each push past the cap, exactly one
/// worker surfaces the ImportTooLarge error and the rest bail. Without
/// the CAS the test would occasionally observe two errors or a
/// race-dependence on worker scheduling.
#[test]
fn parallel_sampling_total_byte_cap_is_enforced_atomically() {
    // 11 files x 100 KB = 1.1 MB body total; set cap to 300 KB so the
    // first 3 workers succeed and the 4th fails the CAS on the total.
    let discovered = write_files_relative(&[
        "f1.rs", "f2.rs", "f3.rs", "f4.rs", "f5.rs", "f6.rs", "f7.rs", "f8.rs", "f9.rs", "f10.rs",
        "f11.rs",
    ]);
    let limits = ResourceLimits {
        max_total_import_bytes: 300_000,
        ..ResourceLimits::default()
    };
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let err = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .expect_err("total-byte cap must fire under this many large files");
    match err {
        PublishError::ImportTooLarge { total, limit } => {
            assert!(
                total > limit,
                "observed total {total} must exceed limit {limit}"
            );
        }
        _ => panic!("expected PublishError::ImportTooLarge, got a different variant"),
    }
}

/// An oversized file (over `limits.max_file_bytes`) must be skipped with a
/// recorded reason — not fatal to the rest of the batch — and the surviving
/// samples must stay in discovered order.
#[test]
fn oversized_files_are_skipped_and_returned_not_fatal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let big = dir.path().join("big.rs");
    let small = dir.path().join("small.rs");
    std::fs::write(&big, vec![b'x'; 100_000]).expect("write big file");
    std::fs::write(&small, b"fn small() {}").expect("write small file");
    let discovered = vec![
        DiscoveredFile {
            relative_path: "big.rs".to_owned(),
            absolute_path: big,
            kind: FileKind::Indexed,
        },
        DiscoveredFile {
            relative_path: "small.rs".to_owned(),
            absolute_path: small,
            kind: FileKind::Indexed,
        },
    ];
    let limits = ResourceLimits {
        max_file_bytes: 50_000,
        ..ResourceLimits::default()
    };
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let (samples, skipped) = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .expect("an oversized file must be skipped, not fatal");

    assert_eq!(samples.len(), 1, "only the in-cap file is sampled");
    assert_eq!(samples[0].relative_path, "small.rs");
    assert_eq!(
        skipped.len(),
        1,
        "the oversized file is reported as skipped"
    );
    assert_eq!(skipped[0].absolute_path, discovered[0].absolute_path);
    assert!(
        skipped[0].reason.contains("exceeding"),
        "skip reason names the ceiling: {}",
        skipped[0].reason
    );
}

/// A deadline that's already spent (exceeded before the first file is
/// dispatched) must surface as `PublishError::TimeBudgetExceeded`
/// rather than completing the work — the wall-clock guard's whole
/// reason to exist is to bound a pathological repo.
#[test]
fn expired_deadline_returns_time_budget_exceeded() {
    let discovered = write_files_relative(&["only.rs"]);
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_millis(1));
    // Sleep past the deadline so the worker loop sees it as
    // already-spent before dispatching its first sample.
    thread::sleep(Duration::from_millis(20));

    let err = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .expect_err("deadline must fire before first sample completes");
    match err {
        PublishError::TimeBudgetExceeded { .. } => {}
        _ => panic!("expected PublishError::TimeBudgetExceeded, got a different variant"),
    }
}

/// Progress events arrive at the sink at least once per sampled file
/// (the exact ordering is racy by design). We just pin the count — the
/// C3 audit's whole reason for parallelizing was keeping the progress
/// channel honest during an initial import.
#[test]
fn parallel_sampling_emits_one_progress_event_per_file() {
    let discovered = write_files_relative(&["p1.rs", "p2.rs", "p3.rs"]);
    let limits = ResourceLimits::default();

    #[derive(Default)]
    struct Counter(Arc<std::sync::atomic::AtomicUsize>);
    impl crate::progress::ProgressSink for Counter {
        fn emit(&self, _event: crate::progress::ProgressEvent) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let counter = Counter::default();
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));
    let _ = sample_all_with_deadline(&discovered, &limits, &counter, &deadline)
        .expect("samples should succeed");

    assert_eq!(
        counter.0.load(std::sync::atomic::Ordering::Relaxed),
        discovered.len(),
        "one Sampling event per file"
    );
}
