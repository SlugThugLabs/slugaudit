use super::*;
use crate::sync::publish::PublishError;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn sample_file_accepts_a_file_exactly_at_the_cap() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("at-cap.rs");
    let body = b"fn at_cap() {}";
    std::fs::write(&path, body).check("write fixture file")?;
    let file = DiscoveredFile {
        relative_path: "at-cap.rs".to_owned(),
        absolute_path: path,
        kind: FileKind::Indexed,
    };
    let limits = ResourceLimits {
        max_file_bytes: body.len() as u64,
        ..ResourceLimits::default()
    };

    let sample = sample_file(&file, &limits).check("the per-file cap is inclusive")?;

    ensure_eq(
        &sample.byte_len,
        &(body.len() as u64),
        "sample.byte_len == body.len() as u64",
    )?;
    ensure_eq(
        &sample.identity,
        &hash::hash_bytes("at-cap.rs", body),
        "sample.identity == hash::hash_bytes(\"at-cap.rs\", body)",
    )?;
    ensure_eq(
        &sample.content.as_deref(),
        &Some("fn at_cap() {}"),
        "sample.content.as_deref() == Some(\"fn at_cap() {}\")",
    )?;
    Ok(())
}

#[test]
fn sample_file_reports_a_file_over_the_cap_as_too_large() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("over.rs");
    std::fs::write(&path, vec![b'x'; 17]).check("write fixture file")?;
    let file = DiscoveredFile {
        relative_path: "over.rs".to_owned(),
        absolute_path: path,
        kind: FileKind::Indexed,
    };
    let limits = ResourceLimits {
        max_file_bytes: 16,
        ..ResourceLimits::default()
    };

    let error = sample_file(&file, &limits).check_err("an over-cap file is refused")?;

    match error {
        SampleError::TooLarge {
            observed, limit, ..
        } => {
            ensure_eq(
                &observed,
                &17,
                "the stat size is reported as a lower bound, not truncated",
            )?;
            ensure_eq(&limit, &16, "limit == 16")?;
        }
        other => {
            return Err(format!("expected SampleError::TooLarge, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}

/// A file can grow between `sample_file`'s stat and its bounded read. The
/// read stops at `cap + 1` bytes, so that length is only a lower bound on
/// the file's size; the skip reason must say so rather than claim an exact
/// size the read never established. A procfs file has stat size 0 but
/// content longer than the cap, which drives the bounded-read arm
/// deterministically without a race.
#[cfg(target_os = "linux")]
#[test]
fn a_file_that_grows_past_the_cap_is_reported_as_a_lower_bound_not_an_exact_size() -> TestResult {
    let file = DiscoveredFile {
        relative_path: "status".to_owned(),
        absolute_path: std::path::PathBuf::from("/proc/self/status"),
        kind: FileKind::Indexed,
    };
    let limits = ResourceLimits {
        max_file_bytes: 10,
        ..ResourceLimits::default()
    };

    let error = sample_file(&file, &limits).check_err("proc status exceeds the cap")?;
    let message = error.to_string();

    match error {
        SampleError::TooLarge {
            observed, limit, ..
        } => {
            ensure_eq(&observed, &11, "the bounded read stops at the cap + 1")?;
            ensure_eq(&limit, &10, "limit == 10")?;
        }
        other => {
            return Err(format!("expected SampleError::TooLarge, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    ensure(
        message.contains("at least 11 bytes"),
        format!("the report must state a lower bound: {message}"),
    )?;
    ensure(
        !message.contains("is 11 bytes"),
        format!("the bounded read length is not the file's size: {message}"),
    )?;
    Ok(())
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
fn write_files_relative(
    relative_paths: &[&str],
) -> TestResult<(tempfile::TempDir, Vec<DiscoveredFile>)> {
    let dir = tempfile::tempdir().check("tempdir")?;
    let root = dir.path().to_path_buf();
    let mut out = Vec::new();
    for rel in relative_paths {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).check("mkdir")?;
        }
        // 100 KB body so the byte-cap test has something concrete to add up.
        let body = vec![b'a'; 100_000];
        std::fs::write(&abs, &body).check("write body")?;
        out.push(DiscoveredFile {
            relative_path: (*rel).to_owned(),
            absolute_path: abs,
            kind: FileKind::Indexed,
        });
    }
    Ok((dir, out))
}

/// Pins that the worker pool still hands back samples indexed by
/// `discovered` order, even though the workers finish out of order.
/// Downstream manifest hashing relies on this; without it, concurrent
/// publishes produce different hashes per run.
#[test]
fn parallel_sampling_preserves_discovered_index_ordering() -> TestResult {
    let (_dir, discovered) = write_files_relative(&[
        "a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs",
    ])?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let (samples, _skipped) = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .check("tight deadline should not fire here")?;

    let actual: Vec<&str> = samples.iter().map(|s| s.relative_path.as_str()).collect();
    let expected: Vec<&str> = discovered
        .iter()
        .map(|d| d.relative_path.as_str())
        .collect();
    ensure_eq(
        &actual,
        &expected,
        "sample indices mirror discovered indices",
    )?;
    Ok(())
}

/// Pins that the concurrent byte-cap check uses CAS, not load+store —
/// i.e. when several workers would each push past the cap, exactly one
/// worker surfaces the `ImportTooLarge` error and the rest bail. Without
/// the CAS the test would occasionally observe two errors or a
/// race-dependence on worker scheduling.
#[test]
fn parallel_sampling_total_byte_cap_is_enforced_atomically() -> TestResult {
    // 11 files x 100 KB = 1.1 MB body total; set cap to 300 KB so the
    // first 3 workers succeed and the 4th fails the CAS on the total.
    let (_dir, discovered) = write_files_relative(&[
        "f1.rs", "f2.rs", "f3.rs", "f4.rs", "f5.rs", "f6.rs", "f7.rs", "f8.rs", "f9.rs", "f10.rs",
        "f11.rs",
    ])?;
    let limits = ResourceLimits {
        max_total_import_bytes: 300_000,
        ..ResourceLimits::default()
    };
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let err = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .check_err("total-byte cap must fire under this many large files")?;
    match err {
        PublishError::ImportTooLarge { total, limit } => {
            ensure(
                total > limit,
                format!("observed total {total} must exceed limit {limit}"),
            )?;
        }
        _ => {
            return Err(
                "expected PublishError::ImportTooLarge, got a different variant".to_owned(),
            )
            .check("unexpected outcome");
        }
    }
    Ok(())
}

/// An oversized file (over `limits.max_file_bytes`) must be skipped with a
/// recorded reason — not fatal to the rest of the batch — and the surviving
/// samples must stay in discovered order.
#[test]
fn oversized_files_are_skipped_and_returned_not_fatal() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let big = dir.path().join("big.rs");
    let small = dir.path().join("small.rs");
    std::fs::write(&big, vec![b'x'; 100_000]).check("write big file")?;
    std::fs::write(&small, b"fn small() {}").check("write small file")?;
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
        .check("an oversized file must be skipped, not fatal")?;

    ensure_eq(&samples.len(), &1, "only the in-cap file is sampled")?;
    ensure_eq(
        &samples.first().check("samples[0] exists")?.relative_path,
        &"small.rs",
        "samples[0].relative_path == \"small.rs\"",
    )?;
    ensure_eq(
        &skipped.len(),
        &1,
        "the oversized file is reported as skipped",
    )?;
    ensure_eq(
        &skipped.first().check("skipped[0] exists")?.absolute_path,
        &discovered
            .first()
            .check("discovered[0] exists")?
            .absolute_path,
        "skipped[0].absolute_path == discovered[0].absolute_path",
    )?;
    ensure(
        skipped
            .first()
            .check("skipped[0] exists")?
            .reason
            .contains("per-file import ceiling"),
        format!(
            "skip reason names the ceiling: {}",
            skipped.first().check("skipped[0] exists")?.reason
        ),
    )?;
    Ok(())
}

/// A deadline that's already spent (exceeded before the first file is
/// dispatched) must surface as `PublishError::TimeBudgetExceeded`
/// rather than completing the work — the wall-clock guard's whole
/// reason to exist is to bound a pathological repo.
#[test]
fn expired_deadline_returns_time_budget_exceeded() -> TestResult {
    let (_dir, discovered) = write_files_relative(&["only.rs"])?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_millis(1));
    // Sleep past the deadline so the worker loop sees it as
    // already-spent before dispatching its first sample.
    thread::sleep(Duration::from_millis(20));

    let err = sample_all_with_deadline(&discovered, &limits, &sink, &deadline)
        .check_err("deadline must fire before first sample completes")?;
    match err {
        PublishError::TimeBudgetExceeded { .. } => {}
        _ => {
            return Err(
                "expected PublishError::TimeBudgetExceeded, got a different variant".to_owned(),
            )
            .check("unexpected outcome");
        }
    }
    Ok(())
}

/// Progress events arrive at the sink at least once per sampled file
/// (the exact ordering is racy by design). We just pin the count — parallel
/// sampling must keep the progress channel honest during an initial import.
#[test]
fn parallel_sampling_emits_one_progress_event_per_file() -> TestResult {
    #[derive(Default)]
    struct Counter(Arc<std::sync::atomic::AtomicUsize>);
    impl crate::progress::ProgressSink for Counter {
        fn emit(&self, _event: crate::progress::ProgressEvent) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let (_dir, discovered) = write_files_relative(&["p1.rs", "p2.rs", "p3.rs"])?;
    let limits = ResourceLimits::default();

    let counter = Counter::default();
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));
    sample_all_with_deadline(&discovered, &limits, &counter, &deadline)
        .check("samples should succeed")?;

    ensure_eq(
        &counter.0.load(std::sync::atomic::Ordering::Relaxed),
        &discovered.len(),
        "one Sampling event per file",
    )?;
    Ok(())
}

/// A path whose stat fails with something other than `NotFound`: a regular
/// file used as a directory component (`ENOTDIR`).
fn unstatable_path(dir: &std::path::Path) -> TestResult<std::path::PathBuf> {
    let plain = dir.join("plain.rs");
    std::fs::write(&plain, b"fn plain() {}\n").check("write the plain file")?;
    Ok(plain.join("inner.rs"))
}

#[test]
fn a_failed_stat_records_an_unknown_size_and_the_stat_failed_reason() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let unstatable = unstatable_path(dir.path())?;

    let record = skipped_record("plain.rs/inner.rs", &unstatable, "oversize")
        .check("a stat failure other than NotFound is recorded, not fatal")?;

    ensure_eq(&record.byte_len, &None, "an unknown size is not 0")?;
    ensure_eq(
        &record.modified_unix_seconds,
        &None,
        "record.modified_unix_seconds == None",
    )?;
    ensure_eq(
        &record.reason,
        &"stat_failed",
        "record.reason == \"stat_failed\"",
    )?;
    Ok(())
}

#[test]
fn a_skipped_file_deleted_since_the_walk_is_a_change_to_the_tree() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let vanished = dir.path().join("vanished.rs");

    let error = skipped_record("vanished.rs", &vanished, "oversize")
        .check_err("a vanished skipped file must not be recorded")?;

    ensure(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "vanished.rs"),
        format!("got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_real_empty_skipped_file_records_zero_bytes_and_keeps_its_reason() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let empty = dir.path().join("empty.rs");
    std::fs::write(&empty, b"").check("write empty file")?;

    let record = skipped_record("empty.rs", &empty, "unreadable").check("record empty file")?;

    ensure_eq(&record.byte_len, &Some(0), "record.byte_len == Some(0)")?;
    ensure_eq(
        &record.reason,
        &"unreadable",
        "record.reason == \"unreadable\"",
    )?;
    let unknown = skipped_record("empty.rs", &unstatable_path(dir.path())?, "unreadable")
        .check("record unstatable file")?;
    ensure_ne(
        &record.identity.content_hash,
        &unknown.identity.content_hash,
        "empty and unknown-size files must not share an identity",
    )?;
    Ok(())
}

#[test]
fn a_skipped_record_for_a_statable_file_does_not_warn() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let present = directory.path().join("present-skip-probe.bin");
    std::fs::write(&present, b"12345").check("write fixture")?;

    let (record, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-probe-skipped-record");
        skipped_record("present-skip-probe.bin", &present, "oversize")
    })?;
    let record = record.check("record a statable file")?;

    ensure_eq(&record.byte_len, &Some(5), "record.byte_len == Some(5)")?;
    ensure(
        logs.contains("control-line-probe-skipped-record"),
        "capture liveness control",
    )?;
    ensure(
        !logs.contains("stat failed for a skipped file"),
        format!("a readable file must not warn:\n{logs}"),
    )?;
    Ok(())
}

/// A sync deadline no test parse comes near.
fn generous_deadline() -> crate::util::Deadline {
    crate::util::Deadline::after(std::time::Duration::from_mins(10))
}

#[path = "evidence_limits_tests.rs"]
mod evidence_limits_tests;
