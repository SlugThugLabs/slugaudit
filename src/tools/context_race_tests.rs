//! Proves `with_verified_read`'s deferred transaction is isolated from a
//! concurrent publish that commits mid-read: once the closure has taken its
//! first snapshot-establishing read, a real `sync::publish` on a second
//! connection must never change what later reads inside the same closure
//! see. `race_hook` (see `src/sync/race_hook.rs`) isn't usable here — its
//! module is private to `sync`, unreachable from `tools` — so this uses a
//! real second thread and a real `mpsc` barrier of its own, pausing inside
//! the closure `with_verified_read` already accepts rather than needing any
//! new test-only hook.
use super::*;
use crate::{parse, store, sync};
use std::fs;
use std::sync::mpsc;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

fn clone_synced(synced: &SyncedProject) -> SyncedProject {
    SyncedProject {
        database_path: synced.database_path.clone(),
        revision_id: synced.revision_id.clone(),
        root: synced.root.clone(),
    }
}

/// A reader parked mid-closure, after its snapshot is established, must see
/// the exact same pre-publish content on every query it issues for the rest
/// of its lifetime — never a torn mix of pre- and post-publish rows.
#[test]
fn a_read_in_progress_never_observes_a_concurrent_publishs_change() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced(&project.path().to_string_lossy(), &SyncRecencyCache::new())
        .expect("initial sync");
    let reader_synced = clone_synced(&synced);

    let (tx_paused, rx_paused) = mpsc::channel::<()>();
    let (tx_resume, rx_resume) = mpsc::channel::<()>();

    let reader = std::thread::spawn(move || {
        with_verified_read(&reader_synced, |tx| {
            // This first query is what actually pins the WAL snapshot (the
            // preceding revision check already ran one, but re-reading here
            // keeps the property under test explicit and self-contained).
            let before: String = tx
                .query_row(
                    "SELECT content FROM files WHERE path = 'lib.rs'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
            let before_count: i64 = tx
                .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

            tx_paused.send(()).expect("signal reader parked");
            rx_resume.recv().expect("wait for the publish to land");

            // Issued after the concurrent publish committed a new revision
            // with different content and an extra file.
            let after: String = tx
                .query_row(
                    "SELECT content FROM files WHERE path = 'lib.rs'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
            let after_count: i64 = tx
                .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

            Ok((before, before_count, after, after_count))
        })
    });

    rx_paused
        .recv()
        .expect("reader reached its parked midpoint");

    // A real concurrent publish, on its own connection, that both modifies
    // the already-read file and adds a brand new one.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    fs::write(project.path().join("new.rs"), b"pub fn b() {}\n").expect("add file");
    let mut connection = store::open_read_write(&synced.database_path).expect("open db (writer)");
    let report = sync::publish(&mut connection, project.path(), parse::PACK_VERSION)
        .expect("concurrent publish succeeds");
    assert_ne!(
        report.revision_id, synced.revision_id,
        "the concurrent publish must actually have moved the revision"
    );
    drop(connection);

    tx_resume.send(()).expect("release the parked reader");
    let (before, before_count, after, after_count) = reader
        .join()
        .expect("reader thread joins")
        .expect("the paused read still succeeds despite the concurrent publish");

    assert_eq!(before, "pub fn a() {}\n");
    assert_eq!(before_count, 1);
    assert_eq!(
        after, before,
        "a read already in progress must keep seeing its original snapshot's content"
    );
    assert_eq!(
        after_count, before_count,
        "a read already in progress must not see the concurrently-published new file"
    );

    // A brand new, independently-synced read does see the new state — this
    // is what confirms the publish really did land, not merely that nothing
    // happened.
    let fresh = ensure_synced(&project.path().to_string_lossy(), &SyncRecencyCache::new())
        .expect("re-sync");
    let fresh_count: i64 = with_verified_read(&fresh, |tx| {
        tx.query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .expect("fresh read succeeds");
    assert_eq!(fresh_count, 2_i64);
}
