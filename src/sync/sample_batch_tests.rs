//! Tests for the parallel sampling pool's worker plumbing.

use super::*;
use crate::sync::test_support::write;

fn one_file_batch<'a>(
    discovered: &'a [crate::sync::discovery::DiscoveredFile],
    limits: &'a ResourceLimits,
    deadline: &'a Deadline,
    total: usize,
) -> Batch<'a> {
    Batch {
        discovered,
        limits,
        deadline,
        total,
        next_index: AtomicUsize::new(0),
        total_bytes: AtomicU64::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    }
}

#[test]
fn a_result_that_cannot_be_delivered_aborts_the_run_with_the_file_named() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"fn a() {}\n");
    let (discovered, _skipped) =
        crate::sync::discovery::discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();
    let deadline = Deadline::after(std::time::Duration::from_secs(60));
    let batch = one_file_batch(&discovered, &limits, &deadline, discovered.len());
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Outcome)>();
    drop(rx);

    let delivered = batch.deliver(&tx, 0, Err("skipped".to_owned()));

    assert!(!delivered, "a closed channel must stop the worker");
    let recorded = lock_or_recover(&batch.error_slot).take();
    assert!(
        matches!(&recorded, Some(PublishError::SamplingResultLost { path, .. }) if path == "lib.rs"),
        "expected SamplingResultLost naming lib.rs, got {recorded:?}"
    );
}

#[test]
fn a_delivery_at_the_end_of_the_discovered_set_fails_with_the_index_error() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"fn a() {}\n");
    let (discovered, _skipped) =
        crate::sync::discovery::discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();
    let deadline = Deadline::after(std::time::Duration::from_secs(60));
    let batch = one_file_batch(&discovered, &limits, &deadline, discovered.len());
    let (tx, _rx) = std::sync::mpsc::channel::<(usize, Outcome)>();

    let delivered = batch.deliver(&tx, discovered.len(), Err("skipped".to_owned()));

    assert!(!delivered, "an out-of-range index must stop the worker");
    let recorded = lock_or_recover(&batch.error_slot).take();
    assert!(
        matches!(
            &recorded,
            Some(PublishError::SamplingIndexOutOfRange { index, total })
                if *index == discovered.len() && *total == discovered.len()
        ),
        "expected SamplingIndexOutOfRange, got {recorded:?}"
    );
}

#[test]
fn a_channel_result_with_an_out_of_range_index_fails_the_run() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"fn a() {}\n");
    let (discovered, _skipped) =
        crate::sync::discovery::discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();
    let deadline = Deadline::after(std::time::Duration::from_secs(60));
    let batch = one_file_batch(&discovered, &limits, &deadline, discovered.len());
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Outcome)>();
    tx.send((discovered.len(), Err("skipped".to_owned())))
        .expect("send the out-of-range result");
    drop(tx);

    let error = batch
        .collect(rx)
        .expect_err("an out-of-range channel index must fail the run");

    assert!(
        matches!(
            error,
            PublishError::SamplingIndexOutOfRange { index, total }
                if index == discovered.len() && total == discovered.len()
        ),
        "expected SamplingIndexOutOfRange, got {error:?}"
    );
}

#[test]
fn a_later_distinct_sampling_error_is_logged_without_leaking_its_path() {
    const SUPERSEDED_PATH: &str = "superseded-second-error-probe.rs";
    let expected_path_id = crate::util::redacted_log_id(SUPERSEDED_PATH);

    let (returned, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-superseded-error-probe");
        let slot = Mutex::new(None::<PublishError>);
        record_error(
            &slot,
            PublishError::ImportTooLarge {
                total: 10,
                limit: 9,
            },
        );
        record_error(
            &slot,
            PublishError::ChangedDuringSample {
                path: SUPERSEDED_PATH.to_owned(),
            },
        );
        lock_or_recover(&slot).take()
    });

    assert!(
        matches!(returned, Some(PublishError::ImportTooLarge { .. })),
        "the first error must stay the returned one: {returned:?}"
    );
    assert!(
        logs.contains("control-line-superseded-error-probe"),
        "the capture must be live: {logs}"
    );
    let logged: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains(&expected_path_id))
        .collect();
    assert_eq!(
        logged.len(),
        1,
        "the later error must be logged exactly once: {logs}"
    );
    assert!(
        logged[0].contains("category=\"changed_during_sample\""),
        "the log must carry the static category: {}",
        logged[0]
    );
    assert!(
        !logs.contains(SUPERSEDED_PATH),
        "the raw path must not be logged: {logs}"
    );
}
