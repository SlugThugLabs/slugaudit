use super::*;
use crate::model::{EvidenceLimits, ExtractionCompleteness};
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
    assert_eq!(
        items
            .iter()
            .map(|item| serde_json::to_vec(&item.payload)
                .expect("payload serializes")
                .len())
            .sum::<usize>(),
        5_326_990,
        "fixture must reproduce the exact reported overflow size"
    );

    let _ = apply_evidence_limits(&mut items, &limits);

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

    let _ = apply_evidence_limits(&mut items, &limits);

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

    let _ = apply_evidence_limits(&mut items, &limits);

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

    let _ = apply_evidence_limits(&mut items, &limits);

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

    let _ = apply_evidence_limits(&mut items, &limits);

    assert!(total_payload_bytes(&items) <= limits.evidence.max_payload_bytes_per_file);
    assert!(items.len() < 10, "later items must have been dropped");
    assert!(has_truncation_marker(&items));
}

/// A file that parsed cleanly is `Full` — the completeness the evidence caps
/// can overclaim. Once the caps discard any item, the stored run must
/// downgrade to `Partial` rather than assert `Full` beside its own truncation
/// marker.
#[test]
fn evidence_limit_truncation_downgrades_a_full_run_to_partial() {
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
        utf8_lossy: false,
        identity: hash::hash_bytes("lib.rs", content.as_bytes()),
        byte_len: content.len() as u64,
        modified_unix_seconds: None,
    };

    let record = to_file_record(sample, &limits, &generous_deadline());

    assert!(has_truncation_marker(&record.evidence));
    assert_eq!(record.run.completeness, ExtractionCompleteness::Partial);
}

/// The downgrade must be conditional: a clean parse with room for every item
/// stays `Full`, so the change cannot simply erase `Full` everywhere.
#[test]
fn a_full_run_within_every_evidence_cap_stays_full() {
    let content = "pub fn a() {}\npub fn b() {}\n";
    let sample = Sample {
        relative_path: "lib.rs".to_owned(),
        is_binary: false,
        content: Some(content.to_owned()),
        utf8_lossy: false,
        identity: hash::hash_bytes("lib.rs", content.as_bytes()),
        byte_len: content.len() as u64,
        modified_unix_seconds: None,
    };

    let record = to_file_record(sample, &ResourceLimits::default(), &generous_deadline());

    assert!(!has_truncation_marker(&record.evidence));
    assert_eq!(record.run.completeness, ExtractionCompleteness::Full);
}

#[test]
fn sample_file_accepts_a_file_exactly_at_the_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("at-cap.rs");
    let body = b"fn at_cap() {}";
    std::fs::write(&path, body).expect("write fixture file");
    let file = DiscoveredFile {
        relative_path: "at-cap.rs".to_owned(),
        absolute_path: path,
        kind: FileKind::Indexed,
    };
    let limits = ResourceLimits {
        max_file_bytes: body.len() as u64,
        ..ResourceLimits::default()
    };

    let sample = sample_file(&file, &limits).expect("the per-file cap is inclusive");

    assert_eq!(sample.byte_len, body.len() as u64);
    assert_eq!(sample.identity, hash::hash_bytes("at-cap.rs", body));
    assert_eq!(sample.content.as_deref(), Some("fn at_cap() {}"));
}

#[test]
fn sample_file_reports_a_file_over_the_cap_as_too_large() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("over.rs");
    std::fs::write(&path, vec![b'x'; 17]).expect("write fixture file");
    let file = DiscoveredFile {
        relative_path: "over.rs".to_owned(),
        absolute_path: path,
        kind: FileKind::Indexed,
    };
    let limits = ResourceLimits {
        max_file_bytes: 16,
        ..ResourceLimits::default()
    };

    let error = sample_file(&file, &limits).expect_err("an over-cap file is refused");

    match error {
        SampleError::TooLarge {
            observed, limit, ..
        } => {
            assert_eq!(
                observed, 17,
                "the stat size is reported as a lower bound, not truncated"
            );
            assert_eq!(limit, 16);
        }
        other => panic!("expected SampleError::TooLarge, got {other:?}"),
    }
}

/// A file can grow between `sample_file`'s stat and its bounded read. The
/// read stops at `cap + 1` bytes, so that length is only a lower bound on
/// the file's size; the skip reason must say so rather than claim an exact
/// size the read never established. A procfs file has stat size 0 but
/// content longer than the cap, which drives the bounded-read arm
/// deterministically without a race.
#[cfg(target_os = "linux")]
#[test]
fn a_file_that_grows_past_the_cap_is_reported_as_a_lower_bound_not_an_exact_size() {
    let file = DiscoveredFile {
        relative_path: "status".to_owned(),
        absolute_path: std::path::PathBuf::from("/proc/self/status"),
        kind: FileKind::Indexed,
    };
    let limits = ResourceLimits {
        max_file_bytes: 10,
        ..ResourceLimits::default()
    };

    let error = sample_file(&file, &limits).expect_err("proc status exceeds the cap");
    let message = error.to_string();

    match error {
        SampleError::TooLarge {
            observed, limit, ..
        } => {
            assert_eq!(observed, 11, "the bounded read stops at the cap + 1");
            assert_eq!(limit, 10);
        }
        other => panic!("expected SampleError::TooLarge, got {other:?}"),
    }
    assert!(
        message.contains("at least 11 bytes"),
        "the report must state a lower bound: {message}"
    );
    assert!(
        !message.contains("is 11 bytes"),
        "the bounded read length is not the file's size: {message}"
    );
}

/// Parallel-sampling tests below. They construct `DiscoveredFile`s
/// directly (synthetic relative paths + temp file bodies) rather than
/// going through the full discovery walker; the goal is to pin
/// `sample_all_with_deadline`'s ordering, cap-enforcement, and deadline
/// behavior in isolation, the way downstream consumers
/// (`manifest::compare`, `aggregate_manifest_hash`,
/// `build_upserts_and_deletions`) depend on it.
/// Writes 100 KB files under a fresh temp directory and returns the
/// directory guard alongside the discovered entries. The caller must keep
/// the guard alive while it uses the paths: dropping it removes the files.
fn write_files_relative(relative_paths: &[&str]) -> (tempfile::TempDir, Vec<DiscoveredFile>) {
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
    (dir, out)
}

/// Pins that the worker pool still hands back samples indexed by
/// `discovered` order, even though the workers finish out of order.
/// Downstream manifest hashing relies on this; without it, concurrent
/// publishes produce different hashes per run.
#[test]
fn parallel_sampling_preserves_discovered_index_ordering() {
    let (_dir, discovered) = write_files_relative(&[
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
    let (_dir, discovered) = write_files_relative(&[
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
        skipped[0].reason.contains("per-file import ceiling"),
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
    let (_dir, discovered) = write_files_relative(&["only.rs"]);
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
    let (_dir, discovered) = write_files_relative(&["p1.rs", "p2.rs", "p3.rs"]);
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

#[test]
fn a_failed_stat_records_an_unknown_size_and_the_stat_failed_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vanished = dir.path().join("vanished.rs");

    let record = skipped_record("vanished.rs", &vanished, "oversize");

    assert_eq!(record.byte_len, None, "an unknown size is not 0");
    assert_eq!(record.modified_unix_seconds, None);
    assert_eq!(record.reason, "stat_failed");
}

#[test]
fn a_real_empty_skipped_file_records_zero_bytes_and_keeps_its_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let empty = dir.path().join("empty.rs");
    std::fs::write(&empty, b"").expect("write empty file");

    let record = skipped_record("empty.rs", &empty, "unreadable");

    assert_eq!(record.byte_len, Some(0));
    assert_eq!(record.reason, "unreadable");
    let unknown = skipped_record("empty.rs", &dir.path().join("gone.rs"), "unreadable");
    assert_ne!(
        record.identity.content_hash, unknown.identity.content_hash,
        "empty and unknown-size files must not share an identity"
    );
}

#[test]
fn a_skipped_record_for_a_statable_file_does_not_warn() {
    let directory = tempfile::tempdir().expect("temp dir");
    let present = directory.path().join("present-skip-probe.bin");
    std::fs::write(&present, b"12345").expect("write fixture");

    let (record, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-probe-skipped-record");
        skipped_record("present-skip-probe.bin", &present, "oversize")
    });

    assert_eq!(record.byte_len, Some(5));
    assert!(
        logs.contains("control-line-probe-skipped-record"),
        "capture liveness control"
    );
    assert!(
        !logs.contains("stat failed for a skipped file"),
        "a readable file must not warn:\n{logs}"
    );
}

#[test]
fn the_truncation_marker_names_items_that_failed_to_serialize_only_when_there_were_some() {
    let without = truncation_evidence(5, 3, 1, 0);
    let with = truncation_evidence(5, 2, 1, 2);

    let message = |item: &EvidenceItem| item.payload["message"].as_str().unwrap_or("").to_owned();
    assert!(!message(&without).contains("failed to serialize"));
    assert!(
        message(&with).contains("2 item(s) failed to serialize"),
        "{}",
        message(&with)
    );
    assert!(
        message(&with).contains("0 item(s) dropped by the item-count"),
        "unmeasurable items must not be double-counted as cap drops: {}",
        message(&with)
    );
}

/// A sync deadline no test parse comes near.
fn generous_deadline() -> crate::util::Deadline {
    crate::util::Deadline::after(std::time::Duration::from_secs(600))
}
