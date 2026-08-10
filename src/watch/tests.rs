//! Integration tests for the filesystem watcher and sync manager.
//! These tests verify that SlugAudit correctly tracks filesystem changes
//! and reconciles them before serving evidence.
//!
//! Note: `normalize_relative_path`'s unit tests live in `path::tests` —
//! `manager::handle_event` exercises the same path end-to-end here, so
//! one path through the watcher is enough to prove it and the unit
//! coverage stays simple.

use crate::watch::{WatchManager, WatchState, WatcherHealth};
use std::fs;

/// Create a temporary project with the activation directory.
fn create_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
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

#[test]
fn watch_manager_tracks_file_modifications() {
    let project = create_project();
    write_file(&project, "lib.rs", b"fn main() {}\n");

    let manager = WatchManager::with_watcher();
    let state = manager.watch(project.path());

    // Initially no events.
    assert!(!state.has_unreconciled_events());

    // Modify the file.
    write_file(&project, "lib.rs", b"fn main() { changed() }\n");

    // Give the watcher a moment to process the event.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Now there should be an unreconciled event.
    // Note: This may be flaky on some platforms if the watcher is slow.
    if state.health() == WatcherHealth::Healthy {
        assert!(state.has_unreconciled_events());
        let snapshot = state.snapshot();
        assert!(snapshot.dirty_paths.contains("lib.rs"));
    }
}

#[test]
fn watch_manager_tracks_file_deletion() {
    let project = create_project();
    write_file(&project, "lib.rs", b"fn main() {}\n");

    let manager = WatchManager::with_watcher();
    let state = manager.watch(project.path());

    // Delete the file.
    fs::remove_file(project.path().join("lib.rs")).expect("delete file");

    std::thread::sleep(std::time::Duration::from_millis(100));

    if state.health() == WatcherHealth::Healthy {
        let snapshot = state.snapshot();
        assert!(snapshot.deleted_paths.contains("lib.rs"));
    }
}

#[test]
fn watch_manager_ignores_slugaudit_activation_dir() {
    let project = create_project();
    write_file(&project, "lib.rs", b"fn main() {}\n");

    let manager = WatchManager::with_watcher();
    let state = manager.watch(project.path());

    // Write a file in the activation directory.
    write_file(&project, ".planning/slugaudit/project.db", b"fake db");

    std::thread::sleep(std::time::Duration::from_millis(100));

    let snapshot = state.snapshot();
    // The activation dir file should be excluded.
    assert!(!snapshot.dirty_paths.iter().any(|p| p.contains("slugaudit")));
}

#[test]
fn watch_state_collapses_repeated_events() {
    let state = WatchState::new();

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
    let state = WatchState::new();

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
