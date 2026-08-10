//! Deterministic coverage for the two publish-time race hazards: a file
//! changing on disk between being sampled and written (TOCTOU), and two
//! independent connections publishing concurrently (CAS). Split out of
//! `publish_tests.rs` to keep both files under the source-size gate.
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

fn stored_paths(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .expect("prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

// --- File-changed-during-sample (TOCTOU) coverage ---

#[test]
fn a_file_changed_after_sampling_is_detected_and_rejected() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"original\n");
    let (discovered, _skipped) = discovery::discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();
    let samples =
        sample_all(&discovered, &limits, &crate::progress::NoopProgressSink).expect("sample");
    // `parser_version_changed: true` forces every sampled file into
    // `upserts` regardless of the (deliberately empty) diff — otherwise an
    // empty `changes` list would make `upserts` empty too, and revalidation
    // over zero files would trivially pass no matter what this test does.
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits);

    // Simulate an editor saving a new version in the gap between sampling
    // and the write that would have published the old sample as current.
    write(project.path(), "lib.rs", b"changed after sampling\n");

    let result = revalidate_unchanged_since_sample(&discovered, &upserts, &limits);
    assert!(
        matches!(&result, Err(PublishError::ChangedDuringSample { path }) if path == "lib.rs"),
        "expected ChangedDuringSample, got {result:?}"
    );
}

#[test]
fn an_unchanged_file_passes_revalidation() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"stable\n");
    let (discovered, _skipped) = discovery::discover(project.path()).expect("discover");
    let limits = ResourceLimits::default();
    let samples =
        sample_all(&discovered, &limits, &crate::progress::NoopProgressSink).expect("sample");
    let (upserts, _deletions) = build_upserts_and_deletions(samples, &[], true, &limits);

    assert!(revalidate_unchanged_since_sample(&discovered, &upserts, &limits).is_ok());
}

#[test]
fn a_file_changed_mid_sample_is_retried_and_the_final_revision_reflects_the_latest_content() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"version one\n");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    let project_path = project.path().to_path_buf();
    race_hook::set(project.path(), move || {
        // Fires once, immediately after the first attempt finishes sampling
        // "version one" but before it revalidates/writes — simulates a
        // concurrent editor save landing in that exact window.
        write(&project_path, "lib.rs", b"version two\n");
    });

    let report = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("publish converges");

    let content: String = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read content");
    assert_eq!(
        content, "version two\n",
        "published content must be the latest sample, not the stale first one"
    );
    assert!(report.revision_id.starts_with("rev-"));
}

// --- Two-connection CAS race coverage ---

#[test]
fn two_connections_publishing_concurrently_the_loser_retries_and_converges() {
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

    let (tx_pause, rx_pause) = std::sync::mpsc::channel::<()>();
    let (tx_resume, rx_resume) = std::sync::mpsc::channel::<()>();
    race_hook::set(project.path(), move || {
        tx_pause.send(()).expect("signal pause");
        rx_resume.recv().expect("wait for resume");
    });

    let project_path = project.path().to_path_buf();
    let db_path_a = db_path.clone();
    let handle = std::thread::spawn(move || {
        write(&project_path, "b.rs", b"fn b() {}\n");
        let mut connection_a = open_read_write(&db_path_a).expect("open db (A)");
        publish(
            &mut connection_a,
            &project_path,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        )
    });

    // A has sampled (a.rs, b.rs) and is now parked in the hook, holding no
    // SQLite lock yet — B is free to publish independently and commit
    // first.
    rx_pause.recv().expect("A reached the barrier");
    write(project.path(), "c.rs", b"fn c() {}\n");
    let mut connection_b = open_read_write(&db_path).expect("open db (B)");
    let report_b = publish(
        &mut connection_b,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("B publishes first");
    drop(connection_b);

    // Release A. Its first attempt must lose the CAS race in
    // `assert_baseline` against B's now-current revision; publish()'s
    // retry loop re-samples (this time seeing c.rs too) and converges.
    tx_resume.send(()).expect("signal resume");
    let report_a = handle
        .join()
        .expect("thread A joins")
        .expect("A converges after retrying");

    assert_eq!(
        report_a.revision_id, report_b.revision_id,
        "A's retry must land on the same revision B already published, not fork a duplicate"
    );
    let connection = open_read_write(&db_path).expect("open db (final)");
    let mut paths = stored_paths(&connection);
    paths.sort();
    assert_eq!(
        paths,
        vec!["a.rs", "b.rs", "c.rs"],
        "no concurrent writer's filesystem change may be lost"
    );
}

#[test]
fn a_noop_retry_after_losing_the_cas_race_converges_on_the_winners_revision() {
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

    let (tx_pause, rx_pause) = std::sync::mpsc::channel::<()>();
    let (tx_resume, rx_resume) = std::sync::mpsc::channel::<()>();
    race_hook::set(project.path(), move || {
        tx_pause.send(()).expect("signal pause");
        rx_resume.recv().expect("wait for resume");
    });

    let project_path = project.path().to_path_buf();
    let db_path_a = db_path.clone();
    let handle = std::thread::spawn(move || {
        // A makes no novel change of its own — everything it eventually
        // reports must come from re-sampling after B's publish.
        let mut connection_a = open_read_write(&db_path_a).expect("open db (A)");
        publish(
            &mut connection_a,
            &project_path,
            "1.0.0",
            &crate::progress::NoopProgressSink,
        )
    });

    rx_pause.recv().expect("A reached the barrier");
    write(project.path(), "b.rs", b"fn b() {}\n");
    let mut connection_b = open_read_write(&db_path).expect("open db (B)");
    let report_b = publish(
        &mut connection_b,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("B publishes");
    drop(connection_b);

    tx_resume.send(()).expect("signal resume");
    let report_a = handle
        .join()
        .expect("thread A joins")
        .expect("A converges without error");

    assert_eq!(
        report_a.revision_id, report_b.revision_id,
        "a no-op retry must return the actual current revision, not error or diverge"
    );
}
