//! Targeted coverage for boundary/control-flow conditions that plain
//! behavioral tests don't pin down precisely enough to catch an off-by-one
//! or operator-swap regression — added after a `cargo-mutants` run against
//! this module found these exact gaps. Split out of publish_tests.rs/
//! publish_race_tests.rs to keep both under the source-size gate.
use super::*;
use crate::store::open_read_write;
use std::fs;

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

#[test]
fn is_retryable_is_true_only_for_the_two_documented_race_hazards() {
    assert!(is_retryable(&PublishError::Revision(
        RevisionError::StaleBaseline {
            expected: "rev-1".into(),
            found: "rev-2".into(),
        }
    )));
    assert!(is_retryable(&PublishError::ChangedDuringSample {
        path: "a.rs".into()
    }));

    assert!(!is_retryable(&PublishError::ImportTooLarge {
        total: 1,
        limit: 0
    }));
    assert!(!is_retryable(&PublishError::Revision(
        RevisionError::InvalidParserRun {
            path: "a.rs".into(),
            reason: "x",
        }
    )));
}

#[test]
fn sample_all_rejects_strictly_over_the_byte_limit_not_at_or_under_it() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "a.rs", b"12345");
    let discovered = discovery::discover(project.path()).expect("discover");

    let exact = ResourceLimits {
        max_total_import_bytes: 5,
        ..ResourceLimits::default()
    };
    assert!(
        sample_all(&discovered, &exact).is_ok(),
        "a total exactly at the limit must be accepted, not rejected"
    );

    let one_under_needed = ResourceLimits {
        max_total_import_bytes: 4,
        ..ResourceLimits::default()
    };
    assert!(
        matches!(
            sample_all(&discovered, &one_under_needed),
            Err(PublishError::ImportTooLarge { .. })
        ),
        "one byte over the limit must be rejected"
    );
}

/// A racer that wins the CAS race *every single time*, re-arming itself
/// after each win: `attempt`'s own connection can never catch up. Pins the
/// exact retry count `publish()` gives up after — not just "it eventually
/// gives up" — which is what actually distinguishes an off-by-one retry
/// bound from the correct one (both look identical for the first N-1
/// races; only the exact Nth matters).
fn arm_permanent_racer(
    root: std::path::PathBuf,
    db_path: std::path::PathBuf,
    attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let hook_root = root.clone();
    race_hook::set(&hook_root, move || {
        let count = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        write(&root, &format!("racer-{count}.rs"), b"fn racer() {}\n");
        let mut racer = open_read_write(&db_path).expect("open racer db");
        publish(&mut racer, &root, "1.0.0").expect("racer publish");
        arm_permanent_racer(root, db_path, attempts);
    });
}

#[test]
fn retry_gives_up_after_exactly_max_cas_retries_and_never_hangs() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "a.rs", b"fn a() {}\n");
    let db_dir = tempfile::tempdir().expect("db dir");
    let db_path = db_dir.path().join("project.db");
    {
        let mut setup = open_read_write(&db_path).expect("open db");
        publish(&mut setup, project.path(), "1.0.0").expect("bootstrap publish");
    }

    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    arm_permanent_racer(
        project.path().to_path_buf(),
        db_path.clone(),
        std::sync::Arc::clone(&attempts),
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let root = project.path().to_path_buf();
    std::thread::spawn(move || {
        let mut connection_a = open_read_write(&db_path).expect("open db (A)");
        let _ = tx.send(publish(&mut connection_a, &root, "1.0.0"));
    });

    // A generous bound: a correct implementation returns almost instantly.
    // If the retry counter's own increment were broken (e.g. never
    // advancing), this call would never return at all — a hang here is
    // itself the failure this test exists to catch, not a flake to retry.
    let result = rx.recv_timeout(std::time::Duration::from_secs(10)).expect(
        "publish() must return within a bounded time even when every attempt \
         loses the CAS race — a hang means the retry counter stopped advancing",
    );

    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        MAX_CAS_RETRIES,
        "must make exactly MAX_CAS_RETRIES attempts before giving up, no more and no fewer"
    );
    assert!(
        matches!(
            result,
            Err(PublishError::Revision(RevisionError::StaleBaseline { .. }))
        ),
        "must give up with a StaleBaseline error after exhausting retries, got {result:?}"
    );
}
