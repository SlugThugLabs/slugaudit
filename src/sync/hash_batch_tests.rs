//! Tests for the parallel streaming-hash worker pool.

use super::*;
use crate::sync::discovery::FileKind;
use crate::sync::test_support::write;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::path::PathBuf;

#[test]
fn hashing_an_empty_file_list_returns_empty_results() -> TestResult {
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let (identities, skips) = hash_all_with_deadline(&[], 1024, &deadline).check("empty hash")?;
    assert!(identities.is_empty());
    assert!(skips.is_empty());
    Ok(())
}

#[test]
fn hashing_normal_files_produces_identities_in_discovery_order() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "a.rs", b"fn a() {}\n")?;
    write(project.path(), "b.rs", b"fn b() {}\n")?;

    let discovered = vec![
        DiscoveredFile {
            relative_path: "a.rs".to_owned(),
            absolute_path: project.path().join("a.rs"),
            kind: FileKind::Indexed,
        },
        DiscoveredFile {
            relative_path: "b.rs".to_owned(),
            absolute_path: project.path().join("b.rs"),
            kind: FileKind::Indexed,
        },
    ];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let (identities, skips) =
        hash_all_with_deadline(&discovered, 1024, &deadline).check("hash batch")?;

    assert_eq!(identities.len(), 2);
    assert_eq!(identities[0].relative_path, "a.rs");
    assert_eq!(identities[1].relative_path, "b.rs");
    assert!(skips.is_empty());
    Ok(())
}

#[test]
fn an_oversized_file_is_skipped_with_oversize_code() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "big.rs", &[b'x'; 20])?;

    let discovered = vec![DiscoveredFile {
        relative_path: "big.rs".to_owned(),
        absolute_path: project.path().join("big.rs"),
        kind: FileKind::Indexed,
    }];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let (identities, skips) =
        hash_all_with_deadline(&discovered, 10, &deadline).check("hash batch")?;

    assert!(identities.is_empty());
    assert_eq!(skips.len(), 1);
    assert_eq!(skips[0].code, SKIP_REASON_OVERSIZE);
    assert!(skips[0].reason.contains("exceeds the 10-byte"));
    Ok(())
}

#[test]
fn an_expired_deadline_aborts_with_time_budget_exceeded() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "slow.rs", b"content\n")?;

    let discovered = vec![DiscoveredFile {
        relative_path: "slow.rs".to_owned(),
        absolute_path: project.path().join("slow.rs"),
        kind: FileKind::Indexed,
    }];
    let deadline = Deadline::after(std::time::Duration::ZERO);
    let error =
        hash_all_with_deadline(&discovered, 1024, &deadline).check_err("deadline must trip")?;

    assert!(matches!(error, PublishError::TimeBudgetExceeded { .. }));
    Ok(())
}

#[test]
fn a_closed_channel_aborts_worker_with_sampling_result_lost() -> TestResult {
    let discovered = [DiscoveredFile {
        relative_path: "drop.rs".to_owned(),
        absolute_path: PathBuf::from("drop.rs"),
        kind: FileKind::Indexed,
    }];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let batch = HashBatch {
        discovered: &discovered,
        max_file_bytes: 1024,
        deadline: &deadline,
        total: 1,
        next_index: AtomicUsize::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    };

    let (tx, rx) = mpsc::channel();
    drop(rx);

    let delivered = batch.send_outcome(
        &tx,
        0,
        HashOutcome::Ok(SourceIdentity {
            relative_path: "drop.rs".to_owned(),
            content_hash: "hash".to_owned(),
            hash_algorithm: "blake3-v1".to_owned(),
        }),
    );
    assert!(!delivered);
    let recorded = crate::util::lock(&batch.error_slot, "test")
        .check("lock the error slot")?
        .take();
    assert!(
        matches!(recorded, Some(PublishError::SamplingResultLost { path, .. }) if path == "drop.rs")
    );
    Ok(())
}

#[test]
fn a_superseded_error_in_hash_batch_is_warn_logged_exactly_once() {
    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        let deadline = Deadline::after(std::time::Duration::from_secs(10));
        let batch = HashBatch {
            discovered: &[],
            max_file_bytes: 1024,
            deadline: &deadline,
            total: 0,
            next_index: AtomicUsize::new(0),
            error_flag: AtomicBool::new(false),
            error_slot: Mutex::new(None),
        };

        batch.abort(PublishError::SamplingResultLost {
            path: "first.rs".to_owned(),
            reason: "closed".to_owned(),
        });
        batch.abort(PublishError::ChangedDuringSample {
            path: "second.rs".to_owned(),
        });
    });

    assert!(logs.contains("a later sampling failure was discarded"));
    assert!(logs.contains("second.rs"));
}
