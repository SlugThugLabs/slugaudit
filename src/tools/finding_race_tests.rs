//! Proves `tools::finding`'s use of `with_verified_write` is not corrupted
//! by a concurrent `sync::publish`: either the finding's write completes
//! against the exact revision it verified, or it detects the staleness via
//! the typed "revision changed concurrently" error `verify_revision_matches`
//! already produces — never a torn or duplicated row. Uses real threads and
//! real second connections, in the same spirit as
//! `src/sync/publish_race_tests.rs`.
use super::*;
use crate::sync;
use crate::tools::context::with_verified_read;
use std::fs;
use std::sync::mpsc;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

fn base_request(project: &tempfile::TempDir, file: &str) -> FindingRequest {
    FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: file.to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "medium".to_owned(),
        category: "correctness".to_owned(),
        title: "Reviewed conclusion".to_owned(),
        description: "A conclusion the AI actually checked.".to_owned(),
    }
}

fn finding_count(project: &tempfile::TempDir) -> i64 {
    let fresh = ensure_synced(&project.path().to_string_lossy()).expect("re-sync");
    with_verified_read(&fresh, |tx| {
        tx.query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .expect("read finding count")
}

/// Deterministic: a full publish is made to land and commit, on its own
/// connection, strictly between the moment a finding write captures its
/// revision and the moment its write transaction actually begins — mirroring
/// exactly what `finding()` itself does (`ensure_synced`, then later
/// `with_verified_write`). The write must detect it verified against a
/// superseded revision and fail with the typed retry error, inserting
/// nothing.
#[test]
fn a_publish_landing_between_revision_capture_and_the_write_is_detected_not_corrupted() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced(&project.path().to_string_lossy()).expect("initial sync");
    let original_revision_id = synced.revision_id.clone();
    let revision_for_insert = synced.revision_id.clone();
    let database_path = synced.database_path.clone();
    let request = base_request(&project, "lib.rs");

    let (tx_ready, rx_ready) = mpsc::channel::<()>();
    let (tx_go, rx_go) = mpsc::channel::<()>();

    let writer = std::thread::spawn(move || {
        tx_ready.send(()).expect("signal the revision was captured");
        rx_go
            .recv()
            .expect("wait for the concurrent publish to land");
        with_verified_write(&synced, |tx| {
            insert_finding(tx, &request, &revision_for_insert)
        })
    });

    rx_ready
        .recv()
        .expect("writer thread captured its revision");

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let mut connection = crate::store::open_read_write(&database_path).expect("open db (writer)");
    let report = sync::publish(&mut connection, project.path(), crate::parse::PACK_VERSION)
        .expect("concurrent publish succeeds");
    assert_ne!(
        report.revision_id, original_revision_id,
        "the concurrent publish must actually have moved the revision"
    );
    drop(connection);

    tx_go.send(()).expect("let the writer proceed");
    let result = writer.join().expect("writer thread joins");

    let error = result.expect_err(
        "a finding write started against a now-superseded revision must fail, \
         not silently insert against the wrong snapshot",
    );
    assert!(
        error.message.contains("revision changed concurrently"),
        "unexpected message: {}",
        error.message
    );
    assert_eq!(
        finding_count(&project),
        0,
        "the closure's INSERT must never have executed once verification failed"
    );
}

/// Genuine concurrent overlap (no imposed ordering beyond a start barrier):
/// a finding write and an unrelated real publish race for real. Whichever
/// wins, the outcome must be one of the two documented cases — a completed
/// finding or the typed retry error — and the findings table must never end
/// up with more than the one row a single successful write could produce.
#[test]
fn a_finding_write_racing_a_real_publish_never_corrupts_state_whichever_wins() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced(&project.path().to_string_lossy()).expect("initial sync");
    let revision_id = synced.revision_id.clone();
    let database_path = synced.database_path.clone();
    let request = base_request(&project, "lib.rs");
    let root = project.path().to_path_buf();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writer_barrier = std::sync::Arc::clone(&barrier);

    let writer = std::thread::spawn(move || {
        writer_barrier.wait();
        with_verified_write(&synced, |tx| insert_finding(tx, &request, &revision_id))
    });

    let publish_root = root.clone();
    let publisher = std::thread::spawn(move || {
        fs::write(publish_root.join("other.rs"), b"pub fn b() {}\n").expect("add unrelated file");
        barrier.wait();
        let mut connection =
            crate::store::open_read_write(&database_path).expect("open db (publisher)");
        sync::publish(&mut connection, &publish_root, crate::parse::PACK_VERSION)
    });

    let write_result = writer.join().expect("writer thread joins");
    let publish_result = publisher.join().expect("publisher thread joins");
    publish_result.expect("publish always converges through its own CAS retry loop");

    match write_result {
        Ok(response) => assert_eq!(response.status, "current"),
        Err(error) => assert!(
            error.message.contains("revision changed concurrently"),
            "a lost race must fail with the typed retry error, not something else: {}",
            error.message
        ),
    }

    let count = finding_count(&project);
    assert!(
        count == 0 || count == 1,
        "expected 0 or 1 finding rows, never a torn/duplicated write, got {count}"
    );
}
