//! Integration tests for the filesystem watcher and sync manager.
//! These tests verify that SlugAudit correctly tracks filesystem changes
//! and reconciles them before serving evidence.
//!
//! Note: `normalize_relative_path`'s unit tests live in `path::tests` —
//! `manager::handle_event` exercises the same path end-to-end here, so
//! one path through the watcher is enough to prove it and the unit
//! coverage stays simple.
//!
//! Event assertions wait for the notify thread instead of sleeping for a
//! fixed 100 ms. A fixed sleep flakes on a loaded machine, and guarding the
//! assertion behind `health == Healthy` made the test pass vacuously: the
//! manager starts a project at `NeedsVerification`, so that branch was
//! never taken. The only precondition is an available watcher; `Unavailable`
//! means the platform has none and the test returns explicitly.

use crate::watch::{WatchManager, WatcherHealth};
use std::fs;
use std::time::{Duration, Instant};

/// How long the notify thread gets to deliver an event before the test
/// fails. Generous enough for a loaded CI machine; a missing event still
/// fails instead of passing silently.
const EVENT_WAIT: Duration = Duration::from_secs(10);

/// Create a temporary project with the activation directory. Created under
/// the canonical temp directory: `watch` keys a project by its canonical
/// root and real watcher events carry canonical paths, while on macOS the
/// default temp dir runs through the `/var -> /private/var` link.
fn create_project() -> tempfile::TempDir {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temp dir");
    let dir = tempfile::tempdir_in(base).expect("create temp dir");
    fs::create_dir_all(dir.path().join(".planning").join("slugaudit"))
        .expect("create activation dir");
    dir
}

/// Write a file in the project.
fn write_file(project: &tempfile::TempDir, relative: &str, content: &[u8]) {
    let path = project.path().join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write file");
}

/// Polls `predicate` until it holds or the event budget expires, and
/// reports the final verdict.
fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + EVENT_WAIT;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

#[test]
fn watch_manager_tracks_file_modifications() {
    let project = create_project();
    write_file(&project, "lib.rs", b"fn main() {}\n");

    let manager = WatchManager::with_watcher();
    let state = manager.watch(project.path(), &Default::default());
    if state.health() == WatcherHealth::Unavailable {
        return;
    }
    // The sync layer moves the state to Healthy after its first full
    // verification; steady-state event tracking is what this test covers.
    state.set_health(WatcherHealth::Healthy);

    write_file(&project, "lib.rs", b"fn main() { changed() }\n");

    assert!(
        wait_until(|| state.snapshot().dirty_paths.contains("lib.rs")),
        "a modify event must be recorded within {EVENT_WAIT:?}; health: {:?}",
        state.health()
    );
}

#[test]
fn watch_manager_tracks_file_deletion() {
    let project = create_project();
    write_file(&project, "lib.rs", b"fn main() {}\n");

    let manager = WatchManager::with_watcher();
    let state = manager.watch(project.path(), &Default::default());
    if state.health() == WatcherHealth::Unavailable {
        return;
    }
    state.set_health(WatcherHealth::Healthy);

    fs::remove_file(project.path().join("lib.rs")).expect("delete file");

    assert!(
        wait_until(|| state.snapshot().deleted_paths.contains("lib.rs")),
        "a delete event must be recorded within {EVENT_WAIT:?}; health: {:?}",
        state.health()
    );
}

#[test]
fn watch_manager_ignores_slugaudit_activation_dir() {
    let project = create_project();
    write_file(&project, "lib.rs", b"fn main() {}\n");

    let manager = WatchManager::with_watcher();
    let state = manager.watch(project.path(), &Default::default());
    if state.health() == WatcherHealth::Unavailable {
        return;
    }
    state.set_health(WatcherHealth::Healthy);

    write_file(&project, ".planning/slugaudit/project.db", b"fake db");
    // Liveness control: a tracked file changed in the same window must be
    // delivered, so the absence assertion below is not vacuously true just
    // because no event reached the manager.
    write_file(&project, "lib.rs", b"fn main() { changed() }\n");
    assert!(
        wait_until(|| state.snapshot().dirty_paths.contains("lib.rs")),
        "the control event must be delivered within {EVENT_WAIT:?}; health: {:?}",
        state.health()
    );

    let snapshot = state.snapshot();
    assert!(
        !snapshot.dirty_paths.iter().any(|p| p.contains("slugaudit")),
        "the activation dir file must never be marked dirty: {:?}",
        snapshot.dirty_paths
    );
    assert!(
        !snapshot
            .deleted_paths
            .iter()
            .any(|p| p.contains("slugaudit")),
        "the activation dir file must never be marked deleted: {:?}",
        snapshot.deleted_paths
    );
}

/// The scope's configured cap is also the kernel-watch bound: registration
/// is one non-recursive watch per scoped directory, so a directory past
/// `max_depth` carries no watch. With a recursive root watch (the previous
/// behavior), notify walked the whole subtree and delivered this file's
/// event — while taking a kernel watch for every directory in it, which is
/// how a large `node_modules/` or `target/` exhausted inotify despite the
/// scope limit.
#[test]
fn a_directory_past_the_scope_depth_cap_carries_no_watch() {
    let project = create_project();
    write_file(&project, "src/live.rs", b"fn live() {}\n");
    write_file(&project, "deep/nested/late.rs", b"fn late() {}\n");

    let manager = WatchManager::with_watcher();
    // max_depth = 1: `src` and `deep` are watched, `deep/nested` is not.
    let state = manager.watch_with_scope_limits_for_test(project.path(), 100, 1);
    if state.health() == WatcherHealth::Unavailable {
        return;
    }
    state.set_health(WatcherHealth::Healthy);

    // Positive control: an event from a scoped directory is delivered.
    write_file(&project, "src/live.rs", b"fn live() { changed }\n");
    assert!(
        wait_until(|| state.snapshot().dirty_paths.contains("src/live.rs")),
        "the in-scope event must be delivered within {EVENT_WAIT:?}; health: {:?}",
        state.health()
    );

    write_file(&project, "deep/nested/late.rs", b"fn late() { changed }\n");
    let grace = Instant::now() + Duration::from_millis(250);
    while Instant::now() < grace {
        assert!(
            !state.snapshot().dirty_paths.contains("deep/nested/late.rs"),
            "a directory past the scope cap must carry no kernel watch: {:?}",
            state.snapshot().dirty_paths
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn watch_state_collapses_repeated_events() {
    let state = crate::watch::WatchState::new();

    // Same file modified multiple times.
    for i in 0..10 {
        let seq = state.mark_dirty(format!("file{}.rs", i % 3));
        assert_eq!(seq, i + 1);
    }

    let (seq, dirty, deleted) = state.snapshot_dirty();
    assert_eq!(seq, 10);
    assert_eq!(dirty.len(), 3); // file0.rs, file1.rs, file2.rs
    assert!(deleted.is_empty());
}

#[test]
fn watch_state_sequence_is_monotonic() {
    let state = crate::watch::WatchState::new();

    let mut prev_seq = 0u64;
    for i in 0..100 {
        let seq = if i % 2 == 0 {
            state.mark_dirty(format!("file{}.rs", i))
        } else {
            state.mark_deleted(format!("file{}.rs", i))
        };
        assert!(seq > prev_seq);
        prev_seq = seq;
    }
}
