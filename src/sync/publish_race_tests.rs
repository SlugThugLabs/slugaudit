//! Deterministic coverage for publish-time race hazards: a file disappearing
//! between discovery and hashing, and two independent connections publishing
//! concurrently (CAS). Kept apart from `publish_tests.rs` to keep both files
//! under the source-size gate.
use super::*;
use crate::store::open_read_write;
use crate::store::test_capture::capture_at_level;
use crate::sync::test_support::{stored_paths, write};
use crate::util::join_checked;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

/// A file the walk listed and that is deleted before Pass 1 hashes it is a
/// change to the tree: the attempt is retried with a fresh walk, and the
/// committed revision does not contain the file. Without the rule the hash
/// pass fails the whole call with a read error.
#[test]
fn a_file_deleted_between_discovery_and_hashing_is_retried_and_left_out_of_the_revision()
-> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "keep.rs", b"fn keep() {}\n")?;
    write(project.path(), "gone.rs", b"fn gone() {}\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let project_path = project.path().to_path_buf();
    let hook = race_hook::set(
        project.path(),
        race_hook::RacePoint::AfterDiscovery,
        move || {
            std::fs::remove_file(project_path.join("gone.rs"))
                .check("delete gone.rs after the walk listed it")
        },
    )?;

    let (result, logs) = capture_at_level(
        || {
            publish(
                &mut connection,
                project.path(),
                "1.0.0",
                &crate::progress::NoopProgressSink,
            )
        },
        tracing::Level::INFO,
    )?;

    let report = result.check("the publish converges after re-walking")?;
    hook.outcome()?;
    ensure_eq(
        &stored_paths(&connection)?,
        &vec!["keep.rs"],
        "stored_paths(&connection)? == vec![\"keep.rs\"]",
    )?;
    ensure_eq(&report.added, &1, "only the file still on disk is added")?;
    ensure(
        report.skipped.is_empty(),
        format!("a deleted file is not a skipped file: {:?}", report.skipped),
    )?;
    let retry_line = logs
        .lines()
        .find(|line| line.contains("publish retrying"))
        .check("the deletion must be retried")?;
    ensure(
        retry_line.contains("error_kind=\"changed_during_sample\"")
            && retry_line.contains("path=gone.rs"),
        format!("got: {retry_line}"),
    )?;
    Ok(())
}

// --- Two-connection CAS race coverage ---

#[test]
fn two_connections_publishing_concurrently_the_loser_retries_and_converges() -> TestResult {
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

    let (tx_pause, rx_pause) = std::sync::mpsc::channel::<()>();
    let (tx_resume, rx_resume) = std::sync::mpsc::channel::<()>();
    let hook = race_hook::set(
        project.path(),
        race_hook::RacePoint::BeforeCommit,
        move || {
            tx_pause.send(()).check("signal pause")?;
            rx_resume.recv().check("wait for resume")
        },
    )?;

    let project_path = project.path().to_path_buf();
    let db_path_a = db_path.clone();
    let handle = std::thread::spawn(move || -> TestResult<_> {
        write(&project_path, "b.rs", b"fn b() {}\n")?;
        let mut connection_a = open_read_write(&db_path_a).check("open db (A)")?;
        Ok(publish(
            &mut connection_a,
            &project_path,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        ))
    });

    // A has sampled (a.rs, b.rs) and is now parked in the hook, holding no
    // SQLite lock yet — B is free to publish independently and commit
    // first.
    rx_pause.recv().check("A reached the barrier")?;
    write(project.path(), "c.rs", b"fn c() {}\n")?;
    let mut connection_b = open_read_write(&db_path).check("open db (B)")?;
    let report_b = publish(
        &mut connection_b,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("B publishes first")?;
    drop(connection_b);

    // Release A. Its first attempt must lose the CAS race in
    // `assert_baseline` against B's now-current revision; publish()'s
    // retry loop re-samples (this time seeing c.rs too) and converges.
    tx_resume.send(()).check("signal resume")?;
    let report_a = join_checked(handle, "thread A joins")??.check("A converges after retrying")?;
    hook.outcome()?;

    ensure_eq(
        &report_a.revision_id,
        &report_b.revision_id,
        "A's retry must land on the same revision B already published, not fork a duplicate",
    )?;
    let connection = open_read_write(&db_path).check("open db (final)")?;
    let mut paths = stored_paths(&connection)?;
    paths.sort();
    ensure_eq(
        &paths,
        &vec!["a.rs", "b.rs", "c.rs"],
        "no concurrent writer's filesystem change may be lost",
    )?;
    Ok(())
}

#[test]
fn a_noop_retry_after_losing_the_cas_race_converges_on_the_winners_revision() -> TestResult {
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

    let (tx_pause, rx_pause) = std::sync::mpsc::channel::<()>();
    let (tx_resume, rx_resume) = std::sync::mpsc::channel::<()>();
    let hook = race_hook::set(
        project.path(),
        race_hook::RacePoint::BeforeCommit,
        move || {
            tx_pause.send(()).check("signal pause")?;
            rx_resume.recv().check("wait for resume")
        },
    )?;

    let project_path = project.path().to_path_buf();
    let db_path_a = db_path.clone();
    let handle = std::thread::spawn(move || -> TestResult<_> {
        // A makes no novel change of its own — everything it eventually
        // reports must come from re-sampling after B's publish.
        let mut connection_a = open_read_write(&db_path_a).check("open db (A)")?;
        Ok(publish(
            &mut connection_a,
            &project_path,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        ))
    });

    rx_pause.recv().check("A reached the barrier")?;
    write(project.path(), "b.rs", b"fn b() {}\n")?;
    let mut connection_b = open_read_write(&db_path).check("open db (B)")?;
    let report_b = publish(
        &mut connection_b,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("B publishes")?;
    drop(connection_b);

    tx_resume.send(()).check("signal resume")?;
    let report_a = join_checked(handle, "thread A joins")??.check("A converges without error")?;
    hook.outcome()?;

    ensure_eq(
        &report_a.revision_id,
        &report_b.revision_id,
        "a no-op retry must return the actual current revision, not error or diverge",
    )?;
    Ok(())
}

/// During Pass 2 (`ProgressEvent::Sampling`), `project.db` must hold no write
/// transaction so concurrent connections can take `BEGIN IMMEDIATE` with zero
/// busy wait while changed files are sampled, parsed, and staged.
#[test]
fn project_db_remains_write_unlocked_throughout_pass2_sampling() -> TestResult {
    struct LockProbeSink {
        db_path: std::path::PathBuf,
        probed: std::sync::Mutex<Option<TestResult>>,
    }

    impl crate::progress::ProgressSink for LockProbeSink {
        fn emit(&self, event: crate::progress::ProgressEvent) {
            if !matches!(event, crate::progress::ProgressEvent::Sampling { .. }) {
                return;
            }
            let Ok(mut slot) = self.probed.lock() else {
                return;
            };
            if slot.is_some() {
                return;
            }
            let outcome = (|| -> TestResult {
                let mut other = open_read_write(&self.db_path).check("open second connection")?;
                other
                    .busy_timeout(std::time::Duration::ZERO)
                    .check("zero busy timeout")?;
                let tx = other
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .check("project.db must not be write-locked during Pass 2")?;
                let count: i64 = tx
                    .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
                    .check("query files under immediate tx")?;
                ensure_eq(&count, &1, "baseline row visible while Pass 2 stages")?;
                tx.rollback().check("rollback probe tx")
            })();
            *slot = Some(outcome);
        }
    }

    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "a.rs", b"pub fn a() {}\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let db_path = db_dir.path().join("project.db");
    let mut connection = open_read_write(&db_path).check("open db")?;
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("initial publish")?;

    write(project.path(), "b.rs", b"pub fn b() {}\n")?;
    let sink = LockProbeSink {
        db_path: db_path.clone(),
        probed: std::sync::Mutex::new(None),
    };
    let report = publish(&mut connection, project.path(), "1.0.0", &sink)
        .check("incremental publish with Pass 2 lock probe")?;
    ensure_eq(&report.added, &1, "b.rs added")?;

    sink.probed
        .lock()
        .check("lock probe result")?
        .take()
        .check("Sampling event fired during Pass 2")?
}
