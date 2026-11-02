//! Deterministic coverage for the two publish-time race hazards: a file
//! changing on disk between being sampled and written (TOCTOU), and two
//! independent connections publishing concurrently (CAS). Kept apart from
//! `publish_tests.rs` to keep both files under the source-size gate.
use super::*;
use crate::store::open_read_write;
use crate::store::test_capture::capture_at_level;
use crate::store::test_join::join_checked;
use crate::sync::test_support::{stored_paths, write};
use slugaudit_test_check::{Check, CheckErr, TestResult};

// --- File-changed-during-sample (TOCTOU) coverage ---

#[test]
fn a_file_changed_after_sampling_is_detected_and_rejected() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"original\n")?;
    let (discovered, _skipped) = discovery::discover(project.path()).check("discover")?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    // A far-future deadline so these tests exercise the race logic, not the
    // wall-clock budget.
    let deadline = crate::util::Deadline::after(std::time::Duration::from_mins(1));
    let (samples, _skipped) =
        sample_all_with_deadline(&discovered, &limits, &sink, &deadline).check("sample")?;
    // `parser_version_changed: true` forces every sampled file into
    // `upserts` regardless of the (deliberately empty) diff — otherwise an
    // empty `changes` list would make `upserts` empty too, and revalidation
    // over zero files would trivially pass no matter what this test does.
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits, &deadline)
        .check("build upserts")?;

    // Simulate an editor saving a new version in the gap between sampling
    // and the write that would have published the old sample as current.
    write(project.path(), "lib.rs", b"changed after sampling\n")?;

    let result = revalidate_unchanged_since_sample(&discovered, &upserts, &limits, &deadline);
    assert!(
        matches!(&result, Err(PublishError::ChangedDuringSample { path }) if path == "lib.rs"),
        "expected ChangedDuringSample, got {result:?}"
    );
    Ok(())
}

#[test]
fn a_file_that_became_unreadable_after_sampling_fails_with_its_real_cause_not_as_a_concurrent_edit()
-> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"original\n")?;
    let (discovered, _skipped) = discovery::discover(project.path()).check("discover")?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(std::time::Duration::from_mins(1));
    let (samples, _skipped) =
        sample_all_with_deadline(&discovered, &limits, &sink, &deadline).check("sample")?;
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits, &deadline)
        .check("build upserts")?;
    // The path becomes a directory: stat succeeds but the read fails with
    // something other than NotFound, which retrying cannot fix.
    std::fs::remove_file(project.path().join("lib.rs")).check("remove the file")?;
    std::fs::create_dir(project.path().join("lib.rs")).check("replace it with a directory")?;

    let result = revalidate_unchanged_since_sample(&discovered, &upserts, &limits, &deadline);

    assert!(
        matches!(
            &result,
            Err(PublishError::Sample(sample::SampleError::Read { .. }))
        ),
        "expected the read error, got {result:?}"
    );
    assert!(!is_retryable(&result.check_err("revalidation fails")?));
    Ok(())
}

#[test]
fn a_file_deleted_after_sampling_is_reported_as_changed_during_sample() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"original\n")?;
    let (discovered, _skipped) = discovery::discover(project.path()).check("discover")?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(std::time::Duration::from_mins(1));
    let (samples, _skipped) =
        sample_all_with_deadline(&discovered, &limits, &sink, &deadline).check("sample")?;
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits, &deadline)
        .check("build upserts")?;
    std::fs::remove_file(project.path().join("lib.rs")).check("remove the file")?;

    let result = revalidate_unchanged_since_sample(&discovered, &upserts, &limits, &deadline);

    assert!(
        matches!(&result, Err(PublishError::ChangedDuringSample { path }) if path == "lib.rs"),
        "a vanished file is a change a fresh sample resolves, got {result:?}"
    );
    Ok(())
}

/// A file that is no longer valid UTF-8 changed since it was sampled as
/// text: revalidation reports the retryable change, not a fatal sampling
/// error whose message claims the file was recorded as skipped.
#[test]
fn a_file_that_stopped_being_utf8_after_sampling_is_reported_as_changed_during_sample() -> TestResult
{
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"original\n")?;
    let (discovered, _skipped) = discovery::discover(project.path()).check("discover")?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(std::time::Duration::from_mins(1));
    let (samples, _skipped) =
        sample_all_with_deadline(&discovered, &limits, &sink, &deadline).check("sample")?;
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits, &deadline)
        .check("build upserts")?;
    // Invalid UTF-8 with no NUL byte, so the discovered kind stays text.
    write(project.path(), "lib.rs", &[0xff, 0xfe, b'a', b'\n'])?;

    let result = revalidate_unchanged_since_sample(&discovered, &upserts, &limits, &deadline);

    assert!(
        matches!(&result, Err(PublishError::ChangedDuringSample { path }) if path == "lib.rs"),
        "a file that is no longer UTF-8 has changed, got {result:?}"
    );
    Ok(())
}

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
    assert_eq!(stored_paths(&connection)?, vec!["keep.rs"]);
    assert_eq!(report.added, 1, "only the file still on disk is added");
    assert!(
        report.skipped.is_empty(),
        "a deleted file is not a skipped file: {:?}",
        report.skipped
    );
    let retry_line = logs
        .lines()
        .find(|line| line.contains("publish retrying"))
        .check("the deletion must be retried")?;
    assert!(
        retry_line.contains("error_kind=\"changed_during_sample\"")
            && retry_line.contains("path=gone.rs"),
        "got: {retry_line}"
    );
    Ok(())
}

#[test]
fn an_unchanged_file_passes_revalidation() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"stable\n")?;
    let (discovered, _skipped) = discovery::discover(project.path()).check("discover")?;
    let limits = ResourceLimits::default();
    let sink = crate::progress::NoopProgressSink;
    let deadline = crate::util::Deadline::after(std::time::Duration::from_mins(1));
    let (samples, _skipped) =
        sample_all_with_deadline(&discovered, &limits, &sink, &deadline).check("sample")?;
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits, &deadline)
        .check("build upserts")?;

    assert!(revalidate_unchanged_since_sample(&discovered, &upserts, &limits, &deadline).is_ok());
    Ok(())
}

#[test]
fn a_file_changed_mid_sample_is_retried_and_the_final_revision_reflects_the_latest_content()
-> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"version one\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    let project_path = project.path().to_path_buf();
    let hook = race_hook::set(
        project.path(),
        race_hook::RacePoint::BeforeCommit,
        move || {
            // Fires once, immediately after the first attempt finishes sampling
            // "version one" but before it revalidates/writes — simulates a
            // concurrent editor save landing in that exact window.
            write(&project_path, "lib.rs", b"version two\n")
        },
    )?;

    let report = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("publish converges")?;
    hook.outcome()?;

    let content: String = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .check("read content")?;
    assert_eq!(
        content, "version two\n",
        "published content must be the latest sample, not the stale first one"
    );
    assert!(report.revision_id.starts_with("rev-"));
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

    assert_eq!(
        report_a.revision_id, report_b.revision_id,
        "A's retry must land on the same revision B already published, not fork a duplicate"
    );
    let connection = open_read_write(&db_path).check("open db (final)")?;
    let mut paths = stored_paths(&connection)?;
    paths.sort();
    assert_eq!(
        paths,
        vec!["a.rs", "b.rs", "c.rs"],
        "no concurrent writer's filesystem change may be lost"
    );
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

    assert_eq!(
        report_a.revision_id, report_b.revision_id,
        "a no-op retry must return the actual current revision, not error or diverge"
    );
    Ok(())
}

/// The retry line must not log `ChangedDuringSample`'s Display, which
/// embeds the changed file's path; it names the retry through the category
/// and a separate `path` field instead.
#[test]
fn a_retry_logs_the_changed_file_under_a_path_field() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"version one\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let project_path = project.path().to_path_buf();
    let hook = race_hook::set(
        project.path(),
        race_hook::RacePoint::BeforeCommit,
        move || write(&project_path, "lib.rs", b"version two\n"),
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

    result.check("the retry converges on the latest content")?;
    hook.outcome()?;
    let retry_line = logs
        .lines()
        .find(|line| line.contains("publish retrying"))
        .check("the changed-during-sample hazard must log a retry")?;
    assert!(
        retry_line.contains("error_kind=\"changed_during_sample\""),
        "got: {retry_line}"
    );
    assert!(
        retry_line.contains("path=lib.rs"),
        "the path must identify the file: {retry_line}"
    );
    Ok(())
}
