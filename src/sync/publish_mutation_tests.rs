//! Targeted coverage for boundary/control-flow conditions that plain
//! behavioral tests don't pin down precisely enough to catch an off-by-one
//! or operator-swap regression — added after a `cargo-mutants` run against
//! this module found these exact gaps. Split out of `publish_tests.rs`/
//! `publish_race_tests.rs` to keep both under the source-size gate.
use super::*;
use crate::store::open_read_write;
use crate::sync::test_support::{stored_paths, write};

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
    let (discovered, _skipped) = discovery::discover(project.path()).expect("discover");

    let generous = crate::util::Deadline::after(std::time::Duration::from_secs(60));
    let exact = ResourceLimits {
        max_total_import_bytes: 5,
        ..ResourceLimits::default()
    };
    assert!(
        sample_all_with_deadline(
            &discovered,
            &exact,
            &crate::progress::NoopProgressSink,
            &generous
        )
        .is_ok(),
        "a total exactly at the limit must be accepted, not rejected"
    );

    let one_under_needed = ResourceLimits {
        max_total_import_bytes: 4,
        ..ResourceLimits::default()
    };
    assert!(
        matches!(
            sample_all_with_deadline(
                &discovered,
                &one_under_needed,
                &crate::progress::NoopProgressSink,
                &generous
            ),
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
        publish(
            &mut racer,
            &root,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        )
        .expect("racer publish");
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
        publish(
            &mut setup,
            project.path(),
            "1.0.0",
            &crate::progress::NoopProgressSink,
        )
        .expect("bootstrap publish");
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
        let _ = tx.send(publish(
            &mut connection_a,
            &root,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        ));
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
        MAX_CAS_RETRIES + 1,
        "must make exactly one initial attempt plus MAX_CAS_RETRIES retries \
         before giving up, no more and no fewer"
    );
    assert!(
        matches!(
            result,
            Err(PublishError::Revision(RevisionError::StaleBaseline { .. }))
        ),
        "must give up with a StaleBaseline error after exhausting retries, got {result:?}"
    );
}

/// A single bad file anywhere in the project (here: a non-UTF8 filename,
/// which is real and filesystem-legal on Unix but can't be stored/queried
/// as text) must not fail the whole publish — every other file must still
/// be discovered, sampled, and committed, with the bad file recorded in
/// `PublishReport::skipped` instead of aborting discovery outright. Uses a
/// non-UTF8 name rather than permission bits so the test is meaningful
/// when run as root (which bypasses permission checks entirely).
#[cfg(unix)]
#[test]
fn one_bad_file_does_not_block_publishing_the_rest_of_the_project() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "good.rs", b"fn good() {}\n");
    let bad_name = OsString::from_vec(vec![b'b', b'a', 0xFF, b'd', b'.', b'r', b's']);
    std::fs::write(project.path().join(&bad_name), b"fn bad() {}\n")
        .expect("write non-UTF-8-named file");

    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    let report = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("publish must succeed despite one bad file");

    assert_eq!(
        report.added, 1,
        "the good file must still be indexed, not blocked by the unrelated bad file"
    );
    assert_eq!(
        report.skipped.len(),
        1,
        "the bad file must be recorded as skipped, not silently dropped or fatal"
    );
}

/// The per-file ceiling mirror of the non-UTF8-name test above: one
/// oversized file must not fail the whole publish. Everything else is
/// still discovered, sampled, and committed, with the oversized file
/// recorded in `PublishReport::skipped` (reason naming the ceiling)
/// instead of aborting the publish outright.
#[test]
fn one_oversized_file_does_not_block_publishing_the_rest_of_the_project() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "good.rs", b"fn good() {}\n");
    std::fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000])
        .expect("write oversized file");

    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    let limits = ResourceLimits {
        max_file_bytes: 50_000,
        ..ResourceLimits::default()
    };
    let report = crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    )
    .expect("publish must succeed despite one oversized file");

    assert_eq!(
        report.added, 2,
        "the good file is indexed and the oversize file gets an existence row; \
         neither is blocked by the other"
    );
    assert_eq!(
        report.skipped.len(),
        1,
        "the oversized file must be recorded as skipped, not silently dropped or fatal"
    );
    assert!(
        report.skipped[0].reason.contains("ceiling"),
        "skip reason names the ceiling: {}",
        report.skipped[0].reason
    );
}

/// A skipped file is on disk, so it must keep its existence row and must
/// never be classified as a deletion — and because that row carries a
/// stat-derived identity, an unchanged tree must still publish nothing on the
/// next call. Without the row in the compared set, every skipped file would
/// look `Deleted` and be re-inserted on each publish; without the stat
/// identity being stable, every publish would churn a revision.
#[test]
fn a_skipped_file_keeps_its_row_and_is_not_deleted_by_the_next_publish() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "good.rs", b"fn good() {}\n");
    std::fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000])
        .expect("write oversized file");

    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    let limits = ResourceLimits {
        max_file_bytes: 50_000,
        ..ResourceLimits::default()
    };

    let first = crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    )
    .expect("first publish");
    assert_eq!(
        first.added, 2,
        "the indexed file and the skipped file's existence row are both new"
    );
    assert_eq!(first.deleted, 0);
    assert_eq!(
        stored_paths(&connection),
        ["good.rs", "huge.rs"],
        "the skipped file must have an existence row"
    );

    let second = crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    )
    .expect("second publish");

    assert_eq!(
        second.revision_id, first.revision_id,
        "an unchanged tree — skipped file included — must not publish a new revision"
    );
    assert_eq!(
        second.deleted, 0,
        "a skipped file exists on disk and must never be classified as deleted"
    );
    assert_eq!(stored_paths(&connection), ["good.rs", "huge.rs"]);
    let (kind, reason): (String, Option<String>) = connection
        .query_row(
            "SELECT file_kind, skip_reason FROM files WHERE path = 'huge.rs'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("skipped row survives");
    assert_eq!(kind, "skipped");
    assert_eq!(reason.as_deref(), Some("oversize"));
}

/// A file that was indexed and then grows past the ceiling must *become* a
/// skipped row: its content, evidence, and completeness all have to be
/// replaced rather than left behind describing bytes nobody retained. This is
/// the overclaim `ContentOnly` exists to prevent, one kind over — a row whose
/// `extraction_completeness` says `Unavailable` sitting beside live `Symbol`
/// rows would contradict itself.
#[test]
fn a_file_that_grows_past_the_ceiling_becomes_a_skipped_row_without_content() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"pub fn a() {}\n");
    write(project.path(), "grow.rs", b"fn a() {}\n");

    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &ResourceLimits::default(),
    )
    .expect("first publish indexes grow.rs");
    let evidence_before: i64 = connection
        .query_row(
            "SELECT count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'grow.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count evidence before");
    assert!(evidence_before > 0, "an indexed file carries evidence");

    std::fs::write(project.path().join("grow.rs"), vec![b'x'; 100_000])
        .expect("grow the file past the ceiling");
    let second = crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &ResourceLimits {
            max_file_bytes: 50_000,
            ..ResourceLimits::default()
        },
    )
    .expect("second publish");

    assert_eq!(second.modified, 1, "grow.rs changed on disk");
    let (kind, reason, content, completeness, outcome): (
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT file_kind, skip_reason, content, extraction_completeness, parse_outcome \
             FROM files WHERE path = 'grow.rs'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("grow.rs row");
    assert_eq!(kind, "skipped");
    assert_eq!(reason.as_deref(), Some("oversize"));
    assert_eq!(content, None, "no content survives the flip to skipped");
    assert_eq!(completeness, "Unavailable");
    assert_eq!(outcome, "NotAttempted");
    let evidence_after: i64 = connection
        .query_row(
            "SELECT count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'grow.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count evidence after");
    assert_eq!(
        evidence_after, 0,
        "the evidence of content nobody retained must be deleted"
    );
    let lib_kind: String = connection
        .query_row(
            "SELECT file_kind FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("lib.rs row");
    assert_eq!(lib_kind, "indexed", "the untouched file is unaffected");
}
