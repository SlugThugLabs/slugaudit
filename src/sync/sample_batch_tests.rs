//! Tests for the parallel sampling pool's worker plumbing.

use super::*;
use crate::sync::test_support::write;

#[test]
fn a_result_that_cannot_be_delivered_aborts_the_run_with_the_file_named() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"fn a() {}\n");
    let (discovered, _skipped) =
        crate::sync::discovery::discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();
    let deadline = Deadline::after(std::time::Duration::from_secs(60));
    let batch = Batch {
        discovered: &discovered,
        limits: &limits,
        deadline: &deadline,
        total: discovered.len(),
        next_index: AtomicUsize::new(0),
        total_bytes: AtomicU64::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    };
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
