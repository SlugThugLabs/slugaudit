//! Tests for the `notify` callback's error path: a watcher error must never
//! block on the manager mutex — a tool thread can hold it across notify's
//! blocking `watch`/`unwatch`, which waits for this callback's event loop —
//! and the queued `Desynced` request must be applied by the next lock
//! holder. Kept as a sibling suite so `manager_event_tests.rs` stays under
//! the test-file line cap.

use super::*;
use crate::watch::WatcherHealth;

/// Created under the canonical temp directory: `watch` keys a project by its
/// canonical root and the manager registers canonical paths.
fn temp_project() -> tempfile::TempDir {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temp dir");
    let dir = tempfile::tempdir_in(base).expect("project dir");
    std::fs::create_dir_all(dir.path().join(".planning").join("slugaudit"))
        .expect("activation dir");
    dir
}

/// R7 regression: the notify error callback must never block on the manager
/// mutex. A tool thread holds it across notify's blocking `watch`/`unwatch`,
/// which waits for the callback's own event loop, so a blocking lock there
/// deadlocks the tool call and the `Desynced` recovery never runs. The
/// error must be queued and applied by the next lock holder instead.
#[test]
fn a_watcher_error_returns_without_the_manager_lock_and_is_applied_on_the_next_drain() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());
    state.set_health(WatcherHealth::Healthy);

    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = manager.clone();
    let holder_thread = std::thread::spawn(move || {
        let guard = holder.inner.lock().expect("hold the manager lock");
        locked_tx.send(()).expect("signal the held lock");
        release_rx.recv().expect("wait for the release");
        drop(guard);
    });
    locked_rx.recv().expect("the holder must take the lock");

    let error = notify::Error::new(notify::ErrorKind::MaxFilesWatch);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let callback = manager.clone();
    let callback_thread = std::thread::spawn(move || {
        callback.handle_notify_error(&error);
        done_tx.send(()).expect("signal the callback returned");
    });
    let returned = done_rx.recv_timeout(std::time::Duration::from_secs(2));

    assert_eq!(
        state.health(),
        WatcherHealth::Healthy,
        "the queued error must wait for the lock, not be lost"
    );
    release_tx.send(()).expect("release the manager lock");
    holder_thread.join().expect("holder thread");
    callback_thread.join().expect("callback thread");
    returned.expect("the notify callback must not block on the manager mutex");

    // The next pass to take the lock drains the queue first — including the
    // read-only `get` that `health` uses, so a health call cannot report the
    // watcher as healthy while the error is still queued.
    let registered = manager
        .get(project.path())
        .expect("the project stays registered");
    assert_eq!(
        registered.health(),
        WatcherHealth::Desynced,
        "the deferred watcher error must mark the project for full verification"
    );
    assert_eq!(state.health(), WatcherHealth::Desynced);
}
