//! Targeted coverage for boundary/control-flow conditions that plain
//! behavioral tests don't pin down precisely enough to catch an off-by-one
//! or operator-swap regression (the gaps a `cargo-mutants` run reports for
//! this module). Kept apart from `publish_tests.rs`/
//! `publish_race_tests.rs` to keep both under the source-size gate.
use super::*;
use crate::store::open_read_write;
use crate::sync::test_support::{stored_paths, write};
use crate::util::join_checked;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn is_retryable_is_true_only_for_the_two_documented_race_hazards() -> TestResult {
    ensure(
        is_retryable(&PublishError::Revision(RevisionError::StaleBaseline {
            expected: "rev-1".into(),
            found: "rev-2".into(),
        })),
        "is_retryable(&PublishError::Revision( RevisionError::StaleBaseline { expected: \"rev-1\".into(), found: \"rev-2\".into(), } ))",
    )?;
    ensure(
        is_retryable(&PublishError::ChangedDuringSample {
            path: "a.rs".into(),
        }),
        "is_retryable(&PublishError::ChangedDuringSample { path: \"a.rs\".into() })",
    )?;

    ensure(
        !is_retryable(&PublishError::ImportTooLarge { total: 1, limit: 0 }),
        "!is_retryable(&PublishError::ImportTooLarge { total: 1, limit: 0 })",
    )?;
    ensure(
        !is_retryable(&PublishError::Revision(RevisionError::InvalidParserRun {
            path: "a.rs".into(),
            reason: "x",
        })),
        "!is_retryable(&PublishError::Revision( RevisionError::InvalidParserRun { path: \"a.rs\".into(), reason: \"x\", } ))",
    )?;
    Ok(())
}

#[test]
fn sample_all_rejects_strictly_over_the_byte_limit_not_at_or_under_it() -> TestResult {
    use crate::sync::sample_batch::{Pass2ChunkContext, run_pass2_chunks};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU64;

    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "a.rs", b"12345")?;
    let (discovered, _skipped) =
        discovery::discover_and_hash(project.path()).check("discover_and_hash")?;

    let generous = crate::util::Deadline::after(std::time::Duration::from_mins(1));
    let exact = ResourceLimits {
        max_total_import_bytes: 5,
        ..ResourceLimits::default()
    };
    let empty_hashes = HashMap::new();
    let total_exact = AtomicU64::new(0);
    let exact_ctx = Pass2ChunkContext {
        total_files: discovered.len(),
        pass1_hashes: &empty_hashes,
        limits: &exact,
        total_bytes: &total_exact,
        sink: &crate::progress::NoopProgressSink,
        deadline: &generous,
    };
    run_pass2_chunks(&discovered, &exact_ctx, |_| Ok(()))
        .check("a total exactly at the limit must be accepted, not rejected")?;

    let one_under_needed = ResourceLimits {
        max_total_import_bytes: 4,
        ..ResourceLimits::default()
    };
    let total_over = AtomicU64::new(0);
    let over_ctx = Pass2ChunkContext {
        total_files: discovered.len(),
        pass1_hashes: &empty_hashes,
        limits: &one_under_needed,
        total_bytes: &total_over,
        sink: &crate::progress::NoopProgressSink,
        deadline: &generous,
    };
    ensure(
        matches!(
            run_pass2_chunks(&discovered, &over_ctx, |_| Ok(())),
            Err(PublishError::ImportTooLarge { .. })
        ),
        "one byte over the limit must be rejected",
    )?;
    Ok(())
}

/// A racer that wins the CAS race *every single time*, re-arming itself
/// after each win: `attempt`'s own connection can never catch up. Pins the
/// exact retry count `publish()` gives up after — not just "it eventually
/// gives up" — which is what actually distinguishes an off-by-one retry
/// bound from the correct one (both look identical for the first N-1
/// races; only the exact Nth matters). Every armed hook's handle is pushed
/// to `handles` so the test can check each racer that fired succeeded.
fn arm_permanent_racer(
    root: std::path::PathBuf,
    db_path: std::path::PathBuf,
    attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handles: &std::sync::Arc<std::sync::Mutex<Vec<race_hook::HookHandle>>>,
) -> TestResult {
    let hook_root = root.clone();
    let hook_handles = std::sync::Arc::clone(handles);
    let handle = race_hook::set(&hook_root, race_hook::RacePoint::BeforeCommit, move || {
        let count = attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .checked_add(1)
            .check("attempt count fits")?;
        write(&root, &format!("racer-{count}.rs"), b"fn racer() {}\n")?;
        let mut racer = open_read_write(&db_path).check("open racer db")?;
        publish(
            &mut racer,
            &root,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        )
        .check("racer publish")?;
        arm_permanent_racer(root, db_path, attempts, &hook_handles)
    })?;
    crate::util::lock(handles, "racer hook handles")
        .check("lock the racer hook handles")?
        .push(handle);
    Ok(())
}

#[test]
fn retry_gives_up_after_exactly_max_cas_retries_and_never_hangs() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "a.rs", b"fn a() {}\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let db_path = db_dir.path().join("project.db");
    {
        let mut setup = open_read_write(&db_path).check("open db")?;
        publish(
            &mut setup,
            project.path(),
            "1.0.0",
            &crate::progress::NoopProgressSink,
        )
        .check("bootstrap publish")?;
    }

    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handles = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    arm_permanent_racer(
        project.path().to_path_buf(),
        db_path.clone(),
        std::sync::Arc::clone(&attempts),
        &handles,
    )?;

    let (tx, rx) = std::sync::mpsc::channel();
    let root = project.path().to_path_buf();
    let worker = std::thread::spawn(move || -> TestResult {
        let mut connection_a = open_read_write(&db_path).check("open db (A)")?;
        tx.send(publish(
            &mut connection_a,
            &root,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        ))
        .check("the test thread holds the receiver until the publish result arrives")
    });

    // A generous bound: a correct implementation returns almost instantly.
    // If the retry counter's own increment were broken (e.g. never
    // advancing), this call would never return at all — a hang here is
    // itself the failure this test exists to catch, not a flake to retry.
    let result = rx.recv_timeout(std::time::Duration::from_secs(10)).check(
        "publish() must return within a bounded time even when every attempt \
         loses the CAS race — a hang means the retry counter stopped advancing",
    )?;

    join_checked(worker, "the publishing thread")??;
    ensure_eq(
        &attempts.load(std::sync::atomic::Ordering::SeqCst),
        &(MAX_CAS_RETRIES + 1),
        "must make exactly one initial attempt plus MAX_CAS_RETRIES retries \
         before giving up, no more and no fewer",
    )?;
    ensure(
        matches!(
            result,
            Err(PublishError::Revision(RevisionError::StaleBaseline { .. }))
        ),
        format!("must give up with a StaleBaseline error after exhausting retries, got {result:?}"),
    )?;
    let handles = std::mem::take(
        &mut *crate::util::lock(&handles, "racer hook handles").check("lock the handles")?,
    );
    // Every attempt fired one racer, and each racer re-armed the next; the
    // last one armed never fires because publish() gave up.
    ensure_eq(
        &handles.len(),
        &(MAX_CAS_RETRIES + 2),
        "handles.len() == MAX_CAS_RETRIES + 2",
    )?;
    for handle in handles.iter().take(MAX_CAS_RETRIES + 1) {
        handle.outcome()?;
    }
    Ok(())
}

/// A single bad file anywhere in the project (here: a non-UTF8 filename,
/// which is real and filesystem-legal on Unix but can't be stored/queried
/// as text) must not fail the whole publish — every other file must still
/// be discovered, sampled, and committed, with the bad file recorded in
/// `PublishReport::skipped` instead of aborting discovery outright. Uses a
/// non-UTF8 name rather than permission bits so the test is meaningful
/// when run as root (which bypasses permission checks entirely).
// APFS, macOS's default filesystem, refuses a non-UTF-8 file name outright
// (`EILSEQ`, "Illegal byte sequence"),
// so this input cannot arise there and the test cannot construct it.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn one_bad_file_does_not_block_publishing_the_rest_of_the_project() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "good.rs", b"fn good() {}\n")?;
    let bad_name = OsString::from_vec(vec![b'b', b'a', 0xFF, b'd', b'.', b'r', b's']);
    std::fs::write(project.path().join(&bad_name), b"fn bad() {}\n")
        .check("write non-UTF-8-named file")?;

    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let report = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("publish must succeed despite one bad file")?;

    ensure_eq(
        &report.added,
        &1,
        "the good file must still be indexed, not blocked by the unrelated bad file",
    )?;
    ensure_eq(
        &report.skipped.len(),
        &1,
        "the bad file must be recorded as skipped, not silently dropped or fatal",
    )?;
    Ok(())
}

/// The per-file ceiling mirror of the non-UTF8-name test above: one
/// oversized file must not fail the whole publish. Everything else is
/// still discovered, sampled, and committed, with the oversized file
/// recorded in `PublishReport::skipped` (reason naming the ceiling)
/// instead of aborting the publish outright.
#[test]
fn one_oversized_file_does_not_block_publishing_the_rest_of_the_project() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "good.rs", b"fn good() {}\n")?;
    std::fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000])
        .check("write oversized file")?;

    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
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
    .check("publish must succeed despite one oversized file")?;

    ensure_eq(
        &report.added,
        &2,
        "the good file is indexed and the oversize file gets an existence row; \
         neither is blocked by the other",
    )?;
    ensure_eq(
        &report.skipped.len(),
        &1,
        "the oversized file must be recorded as skipped, not silently dropped or fatal",
    )?;
    ensure(
        report
            .skipped
            .first()
            .check("report.skipped[0] exists")?
            .reason
            .contains("ceiling"),
        format!(
            "skip reason names the ceiling: {}",
            report
                .skipped
                .first()
                .check("report.skipped[0] exists")?
                .reason
        ),
    )?;
    Ok(())
}

/// A skipped file is on disk, so it must keep its existence row and must
/// never be classified as a deletion — and because that row carries a
/// stat-derived identity, an unchanged tree must still publish nothing on the
/// next call. Without the row in the compared set, every skipped file would
/// look `Deleted` and be re-inserted on each publish; without the stat
/// identity being stable, every publish would churn a revision.
#[test]
fn a_skipped_file_keeps_its_row_and_is_not_deleted_by_the_next_publish() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "good.rs", b"fn good() {}\n")?;
    std::fs::write(project.path().join("huge.rs"), vec![b'x'; 100_000])
        .check("write oversized file")?;

    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
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
    .check("first publish")?;
    ensure_eq(
        &first.added,
        &2,
        "the indexed file and the skipped file's existence row are both new",
    )?;
    ensure_eq(&first.deleted, &0, "first.deleted == 0")?;
    ensure_eq(
        &stored_paths(&connection)?,
        &["good.rs", "huge.rs"],
        "the skipped file must have an existence row",
    )?;

    let second = crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    )
    .check("second publish")?;

    ensure_eq(
        &second.revision_id,
        &first.revision_id,
        "an unchanged tree — skipped file included — must not publish a new revision",
    )?;
    ensure_eq(
        &second.deleted,
        &0,
        "a skipped file exists on disk and must never be classified as deleted",
    )?;
    ensure_eq(
        &stored_paths(&connection)?,
        &["good.rs", "huge.rs"],
        "stored_paths(&connection)? == [\"good.rs\", \"huge.rs\"]",
    )?;
    let (kind, reason): (String, Option<String>) = connection
        .query_row(
            "SELECT file_kind, skip_reason FROM files WHERE path = 'huge.rs'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .check("skipped row survives")?;
    ensure_eq(&kind, &"skipped", "kind == \"skipped\"")?;
    ensure_eq(
        &reason.as_deref(),
        &Some("oversize"),
        "reason.as_deref() == Some(\"oversize\")",
    )?;
    Ok(())
}

/// A file that was indexed and then grows past the ceiling must *become* a
/// skipped row: its content, evidence, and completeness all have to be
/// replaced rather than left behind describing bytes nobody retained. This is
/// the overclaim `ContentOnly` exists to prevent, one kind over — a row whose
/// `extraction_completeness` says `Unavailable` sitting beside live `Symbol`
/// rows would contradict itself.
#[test]
fn a_file_that_grows_past_the_ceiling_becomes_a_skipped_row_without_content() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"pub fn a() {}\n")?;
    write(project.path(), "grow.rs", b"fn a() {}\n")?;

    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    crate::sync::publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &ResourceLimits::default(),
    )
    .check("first publish indexes grow.rs")?;
    let evidence_before: i64 = connection
        .query_row(
            "SELECT count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'grow.rs'",
            [],
            |row| row.get(0),
        )
        .check("count evidence before")?;
    ensure(evidence_before > 0, "an indexed file carries evidence")?;

    std::fs::write(project.path().join("grow.rs"), vec![b'x'; 100_000])
        .check("grow the file past the ceiling")?;
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
    .check("second publish")?;

    ensure_eq(&second.modified, &1, "grow.rs changed on disk")?;
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
        .check("grow.rs row")?;
    ensure_eq(&kind, &"skipped", "kind == \"skipped\"")?;
    ensure_eq(
        &reason.as_deref(),
        &Some("oversize"),
        "reason.as_deref() == Some(\"oversize\")",
    )?;
    ensure_eq(&content, &None, "no content survives the flip to skipped")?;
    ensure_eq(
        &completeness,
        &"Unavailable",
        "completeness == \"Unavailable\"",
    )?;
    ensure_eq(&outcome, &"NotAttempted", "outcome == \"NotAttempted\"")?;
    let evidence_after: i64 = connection
        .query_row(
            "SELECT count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'grow.rs'",
            [],
            |row| row.get(0),
        )
        .check("count evidence after")?;
    ensure_eq(
        &evidence_after,
        &0,
        "the evidence of content nobody retained must be deleted",
    )?;
    let lib_kind: String = connection
        .query_row(
            "SELECT file_kind FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .check("lib.rs row")?;
    ensure_eq(&lib_kind, &"indexed", "the untouched file is unaffected")?;
    Ok(())
}

#[path = "publish_error_class_tests.rs"]
mod error_class;
