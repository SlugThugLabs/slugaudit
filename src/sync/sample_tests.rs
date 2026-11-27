use super::*;
use crate::sync::FileKind;
use crate::sync::publish::PublishError;
use crate::sync::sample_batch::{ChunkOutcome, Pass2ChunkContext, run_pass2_chunks};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::thread;
use std::time::Duration;

fn run_standalone_pass2(
    discovered: &[DiscoveredFile],
    limits: &ResourceLimits,
    sink: &dyn crate::progress::ProgressSink,
    deadline: &Deadline,
) -> Result<(Vec<FileRecord>, Vec<crate::sync::SkippedFile>), PublishError> {
    let empty_hashes = HashMap::new();
    let total_bytes = AtomicU64::new(0);
    let ctx = Pass2ChunkContext {
        total_files: discovered.len(),
        pass1_hashes: &empty_hashes,
        limits,
        total_bytes: &total_bytes,
        sink,
        deadline,
    };
    let mut records = Vec::new();
    let mut skipped = Vec::new();
    run_pass2_chunks(discovered, &ctx, |outcomes| {
        for outcome in outcomes {
            match outcome {
                ChunkOutcome::Parsed(record) => records.push(record),
                ChunkOutcome::Skipped(skip) => skipped.push(skip),
            }
        }
        Ok(())
    })?;
    skipped.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
    Ok((records, skipped))
}

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

#[test]
fn parallel_sampling_preserves_discovered_index_ordering() -> TestResult {
    let (_dir, discovered) = write_files_relative(&[
        "a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt", "g.txt", "h.txt",
    ])?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let (records, _skipped) =
        run_standalone_pass2(&discovered, &limits, &sink, &deadline).check("pass2 succeeds")?;

    let actual: Vec<&str> = records.iter().map(|s| s.relative_path.as_str()).collect();
    let expected: Vec<&str> = discovered
        .iter()
        .map(|d| d.relative_path.as_str())
        .collect();
    ensure_eq(
        &actual,
        &expected,
        "chunked outcomes mirror discovered indices",
    )?;
    Ok(())
}

#[test]
fn parallel_sampling_total_byte_cap_is_enforced_atomically() -> TestResult {
    let (_dir, discovered) = write_files_relative(&[
        "f1.txt", "f2.txt", "f3.txt", "f4.txt", "f5.txt", "f6.txt", "f7.txt", "f8.txt", "f9.txt",
        "f10.txt", "f11.txt",
    ])?;
    let limits = ResourceLimits {
        max_total_import_bytes: 300_000,
        ..ResourceLimits::default()
    };
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));

    let err = run_standalone_pass2(&discovered, &limits, &sink, &deadline)
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

    let (records, skipped) = run_standalone_pass2(&discovered, &limits, &sink, &deadline)
        .check("an oversized file must be skipped, not fatal")?;

    ensure_eq(&records.len(), &1, "only the in-cap file is parsed")?;
    ensure_eq(
        &records.first().check("records[0] exists")?.relative_path,
        &"small.rs",
        "records[0].relative_path == \"small.rs\"",
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

#[test]
fn expired_deadline_returns_time_budget_exceeded() -> TestResult {
    let (_dir, discovered) = write_files_relative(&["only.txt"])?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(Duration::from_millis(1));
    thread::sleep(Duration::from_millis(20));

    let err = run_standalone_pass2(&discovered, &limits, &sink, &deadline)
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

#[test]
fn parallel_sampling_emits_one_progress_event_per_file() -> TestResult {
    #[derive(Default)]
    struct Counter(Arc<std::sync::atomic::AtomicUsize>);
    impl crate::progress::ProgressSink for Counter {
        fn emit(&self, _event: crate::progress::ProgressEvent) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let (_dir, discovered) = write_files_relative(&["p1.txt", "p2.txt", "p3.txt"])?;
    let limits = ResourceLimits::default();

    let counter = Counter::default();
    let deadline = crate::util::Deadline::after(Duration::from_secs(10));
    run_standalone_pass2(&discovered, &limits, &counter, &deadline)
        .check("samples should succeed")?;

    ensure_eq(
        &counter.0.load(std::sync::atomic::Ordering::Relaxed),
        &discovered.len(),
        "one Sampling event per file",
    )?;
    Ok(())
}

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

#[test]
fn sample_file_recomputes_binary_classification_from_actual_bytes() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let path = dir.path().join("changed.rs");
    std::fs::write(&path, b"pub fn text() {}\n").check("write utf8 text")?;
    let stale_binary = DiscoveredFile {
        relative_path: "changed.rs".to_owned(),
        absolute_path: path,
        kind: crate::sync::discovery::FileKind::Binary,
    };

    let sample = sample_file(&stale_binary, &ResourceLimits::default()).check("sample")?;

    ensure(!sample.is_binary, "UTF-8 text must not stay binary")?;
    ensure_eq(
        &sample.content.as_deref(),
        &Some("pub fn text() {}\n"),
        "content is populated from actual bytes",
    )?;
    Ok(())
}
