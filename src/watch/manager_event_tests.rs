//! Deterministic tests for `WatchManager::handle_event` and `unwatch`,
//! using constructed `notify::Event`s instead of waiting on a real
//! filesystem-watcher thread. `handle_event` is the private dispatch that
//! the notify callback thread runs on every event, so exercising it
//! directly removes the timing flakiness of end-to-end watcher tests.

use super::*;
use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};

/// Created under the canonical temp directory: `watch` keys a project by its
/// canonical root and real watcher events carry canonical paths, while on
/// macOS the default temp dir runs through the `/var -> /private/var` link.
fn temp_project() -> tempfile::TempDir {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temp dir");
    let dir = tempfile::tempdir_in(base).expect("project dir");
    std::fs::create_dir_all(dir.path().join(".planning").join("slugaudit"))
        .expect("activation dir");
    dir
}

fn event_with(kind: EventKind, path: &Path) -> Event {
    let mut event = Event::new(kind);
    event.paths.push(path.to_path_buf());
    event
}

#[test]
fn modify_event_marks_the_matching_project_path_dirty() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    let path = project.path().join("src").join("lib.rs");
    manager.handle_event(event_with(EventKind::Modify(ModifyKind::Any), &path));

    let snapshot = state.snapshot();
    assert!(snapshot.dirty_paths.contains("src/lib.rs"));
}

#[test]
fn create_event_marks_the_path_dirty() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    let path = project.path().join("new.rs");
    manager.handle_event(event_with(EventKind::Create(CreateKind::File), &path));

    let snapshot = state.snapshot();
    assert!(snapshot.dirty_paths.contains("new.rs"));
    assert!(snapshot.deleted_paths.is_empty());
}

#[test]
fn remove_event_marks_the_path_deleted() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    let path = project.path().join("lib.rs");
    manager.handle_event(event_with(EventKind::Remove(RemoveKind::File), &path));

    let snapshot = state.snapshot();
    assert!(snapshot.deleted_paths.contains("lib.rs"));
    assert!(snapshot.dirty_paths.is_empty());
}

#[test]
fn access_events_are_ignored() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    let path = project.path().join("lib.rs");
    manager.handle_event(event_with(EventKind::Access(AccessKind::Any), &path));

    let snapshot = state.snapshot();
    assert!(snapshot.dirty_paths.is_empty());
    assert!(snapshot.deleted_paths.is_empty());
}

#[test]
fn excluded_paths_are_not_recorded() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    for relative in [
        ".planning/slugaudit/project.db",
        ".git/config",
        "notes.tmp",
        "notes.bak",
        "buffer.swp",
        "file~",
    ] {
        let path = project.path().join(relative);
        manager.handle_event(event_with(EventKind::Modify(ModifyKind::Any), &path));
    }

    let snapshot = state.snapshot();
    assert!(
        snapshot.dirty_paths.is_empty(),
        "excluded paths must not pollute the dirty set"
    );
    assert!(snapshot.deleted_paths.is_empty());
}

#[test]
fn events_outside_the_project_are_ignored() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    let outside = tempfile::tempdir().expect("outside dir");
    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &outside.path().join("other.rs"),
    ));

    let snapshot = state.snapshot();
    assert!(
        snapshot.dirty_paths.is_empty(),
        "an unrelated path must be ignored"
    );
}

#[test]
fn watch_is_idempotent_and_returns_the_same_state() {
    let project = temp_project();
    let manager = WatchManager::new();

    let first = manager.watch(project.path());
    let second = manager.watch(project.path());

    first.mark_dirty("lib.rs".to_owned());
    assert!(
        second.has_unreconciled_events(),
        "both handles must share one underlying state"
    );
    assert_eq!(manager.iter().len(), 1, "one project registered once");
}

#[test]
fn unwatch_removes_the_state() {
    let project = temp_project();
    let manager = WatchManager::new();
    manager.watch(project.path());
    assert_eq!(manager.iter().len(), 1);

    manager.unwatch(project.path());
    assert!(manager.iter().is_empty(), "unwatch must remove the state");
    assert!(manager.snapshot_all().is_empty());
}

#[test]
fn gitignored_paths_are_not_recorded() {
    let project = temp_project();
    std::fs::write(project.path().join(".gitignore"), "target/\n").expect("gitignore");
    std::fs::create_dir_all(project.path().join("target")).expect("target dir");
    std::fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").expect("target file");
    std::fs::write(project.path().join("lib.rs"), "fn lib() {}\n").expect("lib file");
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join("target/gen.rs"),
    ));
    let snapshot = state.snapshot();
    assert!(
        !snapshot.dirty_paths.contains("target/gen.rs"),
        "a gitignored path must not be marked dirty"
    );

    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join("lib.rs"),
    ));
    assert!(
        state.snapshot().dirty_paths.contains("lib.rs"),
        "an indexable path is still marked dirty"
    );
}

#[test]
fn ignore_file_events_trigger_a_scope_refresh() {
    let project = temp_project();
    std::fs::create_dir_all(project.path().join("src")).expect("src dir");
    std::fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").expect("lib file");
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    // The file is indexable today.
    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join("src/lib.rs"),
    ));
    assert!(state.snapshot().dirty_paths.contains("src/lib.rs"));
    // Drain the dirty set so we can observe fresh events.
    state.snapshot_dirty();

    // Simulate the steady state: the watcher is trusted and current.
    state.set_health(WatcherHealth::Healthy);

    // The project gains a .gitignore that excludes src/. The event for the
    // .gitignore itself must flag the scope for refresh, and after
    // `refresh_scope` runs (as the sync layer does before reconciling) the
    // new rule must filter later events.
    std::fs::write(project.path().join(".gitignore"), "src/\n").expect("gitignore");
    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join(".gitignore"),
    ));
    manager.refresh_scope(project.path());
    assert_eq!(
        state.health(),
        WatcherHealth::NeedsVerification,
        "a scope change must force full verification so newly-ignored \
         files converge out of the database"
    );

    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join("src/lib.rs"),
    ));
    let snapshot = state.snapshot();
    assert!(
        !snapshot.dirty_paths.contains("src/lib.rs"),
        "the refreshed rules must filter new events"
    );
}

#[test]
fn deleted_project_roots_are_pruned_on_the_next_event() {
    let project = temp_project();
    let manager = WatchManager::new();
    manager.watch(project.path());
    assert_eq!(manager.iter().len(), 1);

    // The project root disappears; the next event prunes its stale entry.
    std::fs::remove_dir_all(project.path()).expect("remove project root");
    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join("lib.rs"),
    ));

    assert!(
        manager.iter().is_empty(),
        "a deleted project root must be pruned, not accumulate"
    );
    assert!(manager.snapshot_all().is_empty());
}

#[test]
fn an_event_arriving_while_the_lock_is_held_is_applied_by_the_next_watch_call() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());

    let held = manager.inner.lock().expect("hold the manager lock");
    let path = project.path().join("src").join("lib.rs");
    manager.handle_event(event_with(EventKind::Modify(ModifyKind::Any), &path));
    drop(held);
    assert!(
        state.snapshot().dirty_paths.is_empty(),
        "the notify thread must not block, so the event waits"
    );
    manager.watch(project.path());

    assert!(
        state.snapshot().dirty_paths.contains("src/lib.rs"),
        "a deferred event must be recorded, not lost"
    );
}

#[test]
fn deferred_events_are_applied_in_arrival_order_before_a_later_live_event() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());
    let path = project.path().join("lib.rs");

    let held = manager.inner.lock().expect("hold the manager lock");
    manager.handle_event(event_with(EventKind::Modify(ModifyKind::Any), &path));
    drop(held);
    manager.handle_event(event_with(EventKind::Remove(RemoveKind::File), &path));

    let snapshot = state.snapshot();
    assert!(
        snapshot.deleted_paths.contains("lib.rs") && snapshot.dirty_paths.is_empty(),
        "the later delete must supersede the earlier deferred modify"
    );
}

#[test]
fn overflowing_the_deferred_queue_desyncs_the_project_instead_of_losing_events_silently() {
    let project = temp_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());
    state.set_health(WatcherHealth::Healthy);
    let path = project.path().join("lib.rs");

    let held = manager.inner.lock().expect("hold the manager lock");
    for _ in 0..=MAX_DEFERRED_EVENTS {
        manager.handle_event(event_with(EventKind::Modify(ModifyKind::Any), &path));
    }
    drop(held);
    manager.watch(project.path());

    assert_eq!(
        state.health(),
        WatcherHealth::Desynced,
        "events past the queue cap must force a full re-verification"
    );
}

/// A root whose only watchable directory is the root itself: `.git` is a
/// hard exclusion and `lib.rs` is a file, so a truncated scope still has
/// the same watch set as the full one. That isolates the health transition
/// from the changed-watch-set rule (which forces `NeedsVerification`).
fn root_only_scope_project() -> tempfile::TempDir {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temp dir");
    let project = tempfile::tempdir_in(base).expect("project dir");
    std::fs::write(project.path().join("lib.rs"), "fn lib() {}\n").expect("lib file");
    std::fs::create_dir_all(project.path().join(".git")).expect("git dir");
    std::fs::write(project.path().join(".git/config"), "[core]\n").expect("git config");
    project
}

fn mark_scope_for_refresh(manager: &WatchManager, project: &tempfile::TempDir) {
    manager.handle_event(event_with(
        EventKind::Modify(ModifyKind::Any),
        &project.path().join(".gitignore"),
    ));
}

#[test]
fn a_truncated_scope_moves_a_healthy_project_to_partial() {
    let project = root_only_scope_project();
    let manager = WatchManager::new();
    let state = manager.watch(project.path());
    state.set_health(WatcherHealth::Healthy);

    mark_scope_for_refresh(&manager, &project);
    manager.refresh_scope_with_limits_for_test(project.path(), 1, 64);

    assert!(
        manager.scope_limited(project.path()),
        "one entry of a multi-entry tree must produce a limited scope"
    );
    assert_eq!(
        state.health(),
        WatcherHealth::Partial,
        "a limited scope on an unchanged watch set is Partial, not Healthy"
    );
}

#[test]
fn a_scope_that_is_no_longer_truncated_moves_partial_back_to_healthy() {
    let project = root_only_scope_project();
    let manager = WatchManager::new();
    let state = manager.watch_with_scope_limits_for_test(project.path(), 1, 64);
    assert!(manager.scope_limited(project.path()));
    state.set_health(WatcherHealth::Partial);

    mark_scope_for_refresh(&manager, &project);
    manager.refresh_scope_with_limits_for_test(project.path(), 1_000_000, 64);

    assert!(!manager.scope_limited(project.path()));
    assert_eq!(
        state.health(),
        WatcherHealth::Healthy,
        "a complete scope restores Healthy when the watch set did not change"
    );
}

/// The walk is the expensive half of a scope refresh. It must not run under
/// the manager lock: if it did, every tool call and the notify callback
/// would wait for a whole-tree walk. The hook runs on the walking thread
/// (this one), so a held lock would be held by this very thread and
/// `try_lock` would fail deterministically — no timing guess is involved.
#[test]
fn the_scope_walk_runs_without_holding_the_manager_lock() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let project = temp_project();
    let manager = WatchManager::new();
    let probe = Arc::new(AtomicBool::new(false));
    let probe_in_hook = Arc::clone(&probe);
    let manager_in_hook = manager.clone();

    crate::watch::set_walk_hook(Box::new(move || {
        probe_in_hook.store(manager_in_hook.inner.try_lock().is_ok(), Ordering::SeqCst);
    }));

    manager.watch(project.path());

    assert!(
        probe.load(Ordering::SeqCst),
        "the manager lock must be free while the scope walk runs; a tool \
         call arriving mid-walk must not block on the walk"
    );
}
