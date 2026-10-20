//! Tests for `SourceSyncManager`'s observability surface: the
//! last-sync timestamp, watcher-state accessors, and the direct
//! `reconcile` entry point. Kept separate from `manager_tests.rs` so
//! each file stays under the source-size cap.

use crate::sync::SourceSyncManager;
use crate::sync::race_hook;
use crate::sync::test_support::{create_project, sync_project, write_file};
use crate::watch::WatcherHealth;
use std::fs;

fn open_db(project: &tempfile::TempDir) -> rusqlite::Connection {
    let database = project.path().join(".planning/slugaudit/project.db");
    crate::store::open_read_write(&database).expect("open db")
}

/// Removes a file, tolerating its absence. Used by the corruption fixtures
/// to clear SQLite sidecars before replacing one with a directory.
fn remove_if_present(path: &std::path::Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove {}: {error}", path.display()),
    }
}

/// The stored content of one indexed file, for tests that assert the
/// reconcile landed.
fn content_of(connection: &rusqlite::Connection, path: &str) -> Option<String> {
    connection
        .query_row("SELECT content FROM files WHERE path = ?1", [path], |row| {
            row.get(0)
        })
        .expect("read file content")
}

#[test]
fn last_sync_timestamp_is_zero_before_first_sync_and_stamped_after() {
    let manager = SourceSyncManager::new();
    assert_eq!(manager.last_sync_unix_seconds(), 0, "nothing synced yet");
    assert_eq!(manager.last_sync_duration_ms(), 0, "nothing synced yet");

    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    assert!(
        manager.last_sync_unix_seconds() > 0,
        "a successful ensure_current must stamp the timestamp"
    );
}

#[test]
fn watcher_state_accessors_move_from_empty_to_registered() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    assert!(manager.active_watch_state().is_none(), "no project yet");
    assert!(manager.watch_states_snapshot().is_empty());

    sync_project(&manager, &project);

    assert!(manager.active_watch_state().is_some(), "now registered");
    assert_eq!(manager.watch_states_snapshot().len(), 1);

    let root = project.path().canonicalize().expect("canonical root");
    let state = manager.watch_state_for(&root).expect("registered state");
    assert_eq!(
        state.health(),
        WatcherHealth::Unavailable,
        "a watcher-less manager reports Unavailable"
    );

    let other = tempfile::tempdir().expect("other dir");
    assert!(
        manager.watch_state_for(other.path()).is_none(),
        "an unknown root has no watch state"
    );
}

#[test]
fn reconcile_with_no_pending_events_is_a_noop() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    let state = manager.active_watch_state().expect("registered state");
    let mut connection = open_db(&project);
    manager
        .reconcile(project.path(), &state, &mut connection)
        .expect("reconcile with no events");

    let revision: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .expect("current revision");
    assert!(
        !revision.is_empty(),
        "the current revision survives a no-op reconcile"
    );
}

#[test]
fn reconcile_dirty_unchanged_paths_skips_revision_churn() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    let state = manager.active_watch_state().expect("registered state");
    state.mark_dirty("lib.rs".to_owned());
    let mut connection = open_db(&project);
    manager
        .reconcile(project.path(), &state, &mut connection)
        .expect("reconcile unchanged path");

    // The dirty path hashes to the stored hash, so no revision is
    // published.
    let count: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("revision count");
    assert_eq!(count, 1, "no new revision for an unchanged dirty path");
    assert_eq!(state.health(), WatcherHealth::Unavailable);
}

/// Arms a one-shot hook that corrupts the database mid-publish (after
/// sampling, before the diff/revision writes), so the publish fails with a
/// non-retryable database error. Deleting a source file instead would be
/// retryable (publish retries and indexes the deletion); corrupting the DB
/// cannot be retried into success. `ensure_current` must surface the
/// failure as an `ErrorData`, never panic and never leave a half-published
/// revision behind.
fn corrupt_database_hook(project: &tempfile::TempDir) {
    let project_path = project.path().to_path_buf();
    let hook_path = project_path.clone();
    race_hook::set(&project_path, move || {
        let database = hook_path.join(".planning/slugaudit/project.db");
        fs::write(database, b"corrupted mid-publish").expect("corrupt the database mid-publish");
    });
}

#[test]
fn a_failing_publish_is_surfaced_as_an_error_not_a_panic() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    corrupt_database_hook(&project);
    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );

    assert!(
        result.is_err(),
        "a publish that fails mid-flight must surface as an error"
    );
}

/// Same guarantee for the watcher-backed verification path: a fresh
/// manager starts `NeedsVerification`, and a full-verification publish
/// that fails must surface as an error rather than a panic.
#[test]
fn a_failing_verification_publish_is_surfaced_as_an_error() {
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let manager1 = SourceSyncManager::with_watcher();
    sync_project(&manager1, &project);
    drop(manager1);

    let manager2 = SourceSyncManager::with_watcher();
    corrupt_database_hook(&project);
    let result = manager2.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );

    assert!(
        result.is_err(),
        "a failed verification publish must surface as an error"
    );
}

/// A corrupt on-disk database must be discarded and rebuilt from scratch on
/// the next sync rather than erroring forever — the project stays usable.
/// Exercises `ensure_current`'s corruption-at-open branch (discard +
/// `publish_from_scratch`).
#[test]
fn a_corrupt_database_is_discarded_and_rebuilt_on_the_next_sync() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);

    // Replace the database with bytes that are not a SQLite database at
    // all, so the next open fails as corrupt.
    let database = project.path().join(".planning/slugaudit/project.db");
    fs::write(&database, b"this is not a sqlite database").expect("corrupt db");

    // ensure_current must discard the corrupt database and rebuild from
    // scratch instead of erroring — the project stays usable. (The rebuilt
    // database's first revision id is `rev-1` again: ids are per-database
    // rowids, so the revision id is not a uniqueness signal across
    // incarnations.)
    let _synced2 = sync_project(&manager, &project);
    let connection = open_db(&project);
    let content: Option<String> = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read content from the rebuilt database");
    assert!(
        content.expect("read file back").contains("pub fn a()"),
        "the rebuilt index must still serve the file's evidence"
    );
}

/// A database carried into a *different* project root (repo moved, copied,
/// or re-extracted from an archive) must be auto-discarded and rebuilt from
/// the current source rather than blocking the tool call with a
/// per-project-root error. This is the papercut the Hermes integration run
/// surfaced: unzipping a GitHub repo brings `.planning/slugaudit/project.db`
/// along, and its stored root no longer matches the new extraction path.
#[test]
fn a_stale_project_root_is_discarded_and_rebuilt_on_the_next_sync() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);

    // Simulate a move/copy: rewrite the stored root so it no longer matches
    // the canonical root resolve_project produces. (The real trigger is the
    // project being accessed from a new location; here we just corrupt the
    // metadata row to say it was built somewhere else.)
    let database = project.path().join(".planning/slugaudit/project.db");
    let stale_root = format!(
        "{}-moved",
        project
            .path()
            .canonicalize()
            .expect("canonicalize project tempdir")
            .display()
    );
    {
        let conn = crate::store::open_read_write(&database).expect("open db");
        conn.execute(
            "UPDATE project SET root_path = ?1 WHERE id = 1",
            [stale_root],
        )
        .expect("rewrite stored root");
    }

    // ensure_current must discard the stale db and rebuild from the current
    // source instead of erroring — the project stays usable.
    // Must auto-recover, not error — sync_project unwraps, so this proves
    // the stale root mismatch was recovered rather than surfaced.
    let synced2 = sync_project(&manager, &project);

    let connection = open_db(&project);
    let stored: String = connection
        .query_row("SELECT root_path FROM project WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("read stored root");
    assert!(
        !stored.contains("-moved"),
        "the stale root must have been replaced, got: {stored}"
    );
    let content: Option<String> = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read content from the rebuilt database");
    assert!(
        content.expect("read file back").contains("pub fn a()"),
        "the rebuilt index must still serve the file's evidence"
    );
    // A stale root mismatch is recovered by discarding and re-publishing, so a
    // brand-new revision is published rather than an incremental one.
    assert_eq!(synced2.revision_id, "rev-1");
}

/// A database written by a newer/incompatible SlugAudit (unsupported
/// contract/schema version) must be treated like any other stale derived
/// data: discard and rebuild from the current source, not hard-fail.
#[test]
fn an_incompatible_version_database_is_discarded_and_rebuilt() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);

    // Simulate a DB built by a different SlugAudit version.
    let database = project.path().join(".planning/slugaudit/project.db");
    {
        let conn = crate::store::open_read_write(&database).expect("open db");
        conn.execute(
            "UPDATE project SET contract_version = 999, schema_version = 999 WHERE id = 1",
            [],
        )
        .expect("rewrite stored version");
    }

    // Must auto-recover (discard + rebuild), not error.
    let synced2 = sync_project(&manager, &project);
    let connection = open_db(&project);
    let (contract_version, schema_version): (i64, i64) = connection
        .query_row(
            "SELECT contract_version, schema_version FROM project WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read rebuilt version");
    assert_eq!(
        contract_version, 1,
        "rebuilt to the current contract version"
    );
    assert_eq!(schema_version, 1, "rebuilt to the current schema version");
    let content: Option<String> = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read content from the rebuilt database");
    assert!(
        content.expect("read file back").contains("pub fn a()"),
        "the rebuilt index must still serve the file's evidence"
    );
    assert_eq!(synced2.revision_id, "rev-1");
}

/// A database stamped with the previous schema version (6, before
/// `files.byte_len` became nullable) must be discarded and rebuilt at the
/// current version, with no migration code.
#[test]
fn a_database_at_the_previous_schema_version_is_rebuilt_at_the_current_one() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let _synced = sync_project(&manager, &project);
    let database = project.path().join(".planning/slugaudit/project.db");
    {
        let conn = crate::store::open_read_write(&database).expect("open db");
        conn.pragma_update(
            None,
            "user_version",
            crate::store::CURRENT_SCHEMA_VERSION - 1,
        )
        .expect("stamp the previous schema version");
    }

    let synced = sync_project(&manager, &project);

    let connection = open_db(&project);
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read user_version");
    assert_eq!(version, crate::store::CURRENT_SCHEMA_VERSION);
    assert_eq!(synced.revision_id, "rev-1", "the database was rebuilt");
}

/// An unopenable database (the db path is a directory) must surface as an
/// error, never a panic, and never silently publish into nothing.
#[test]
fn an_unopenable_database_surfaces_as_an_error_not_a_panic() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");

    let database = project.path().join(".planning/slugaudit/project.db");
    fs::create_dir_all(&database).expect("make the db path a directory");
    // The dev-only permission guard would otherwise reject the 0755
    // directory before SQLite is asked to open it; owner-only makes the
    // failure the SQLite open itself, which is the arm under test.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))
            .expect("restrict the database directory to owner-only");
    }

    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    let error = match result {
        Ok(_) => panic!("an unopenable database must error, not panic"),
        Err(error) => error,
    };
    // The store's own error is the only place the platform-specific SQLite
    // text and path live; asserting the exact wrapper around it proves the
    // non-corruption arm surfaces the open failure rather than a generic
    // message or a different error source.
    let open_error = crate::store::open_read_write(&database)
        .expect_err("the database path is still a directory");
    assert_eq!(
        error.message,
        format!("opening the project database for sync: {open_error}"),
        "the exact non-corruption open failure must reach the caller"
    );
    assert!(
        error.data.is_none(),
        "an unopenable database is not a coded, recoverable condition"
    );
}

/// When the watcher is Healthy and incremental reconcile fails, the
/// watcher must be marked `Desynced` (so the next call does a full
/// verification) and the failure surfaced — never a silent serve of stale
/// evidence. A trigger that aborts every write to `files` makes the
/// reconcile's publish fail deterministically, while the stat sweep before
/// it only reads, so the failure lands in the drain and nowhere earlier.
#[test]
fn a_failed_incremental_reconcile_marks_the_watcher_desynced() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let synced = sync_project(&manager, &project);
    rusqlite::Connection::open(&synced.database_path)
        .expect("open database")
        .execute_batch(
            "CREATE TRIGGER abort_file_insert BEFORE INSERT ON files \
             BEGIN SELECT RAISE(ABORT, 'injected reconcile failure'); END; \
             CREATE TRIGGER abort_file_update BEFORE UPDATE ON files \
             BEGIN SELECT RAISE(ABORT, 'injected reconcile failure'); END;",
        )
        .expect("install failing triggers");
    write_file(&project, "lib.rs", b"pub fn a_changed() {}\n");

    let state = manager.active_watch_state().expect("registered state");
    state.mark_dirty("lib.rs".to_owned());

    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    let error = match result {
        Ok(_) => panic!("a failed reconcile must surface as an error"),
        Err(error) => error,
    };
    assert_eq!(
        error.message,
        "reconciling watcher events: reconcile error: database error: \
         injected reconcile failure",
        "the exact drain failure must reach the caller"
    );
    assert!(
        error.data.is_none(),
        "a database failure is not a coded budget condition"
    );
    assert_eq!(
        state.health(),
        WatcherHealth::Desynced,
        "the watcher must be marked untrusted so the next call re-verifies"
    );
}

/// A second event that arrives while the first barrier iteration is
/// publishing must be reconciled in the same call. The barrier loop runs
/// another iteration when the watcher sequence advances, and that
/// iteration must publish against the revision the prior iteration just
/// produced — not a baseline read before the loop. Otherwise a change the
/// call could reconcile fails `StaleBaseline` and (on the Healthy path)
/// marks the watcher `Desynced`.
#[test]
fn an_event_arriving_during_a_barrier_publish_is_reconciled_in_the_same_call() {
    let manager = SourceSyncManager::with_watcher();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    write_file(&project, "second.rs", b"pub fn second_v1() {}\n");
    let _synced = sync_project(&manager, &project);
    let state = manager
        .watch_state_for(project.path())
        .expect("the synced project is registered");
    assert_eq!(state.health(), WatcherHealth::Healthy);

    let mut connection = open_db(&project);
    // The commit hook fires inside iteration 1's publish transaction —
    // after the revision is written, before the barrier loop checks for
    // new events. That is exactly the window an editor's save lands in,
    // and unlike a background thread it is deterministic.
    let hook_root = project.path().to_path_buf();
    let hook_state = state.clone();
    let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_fired = std::sync::Arc::clone(&fired);
    connection.commit_hook(Some(move || {
        if !hook_fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
            fs::write(hook_root.join("second.rs"), b"pub fn second_v2() {}\n")
                .expect("write the second file while the first publish commits");
            hook_state.mark_dirty("second.rs".to_owned());
        }
        // rusqlite's contract: returning true rolls the commit back.
        false
    }));

    write_file(&project, "lib.rs", b"pub fn a_changed() {}\n");
    state.mark_dirty("lib.rs".to_owned());

    manager
        .reconcile(project.path(), &state, &mut connection)
        .expect("both events must reconcile in one call");

    assert!(
        fired.load(std::sync::atomic::Ordering::SeqCst),
        "hook did not fire"
    );
    assert_eq!(state.health(), WatcherHealth::Healthy, "must stay Healthy");
    let lib_rs = content_of(&connection, "lib.rs").expect("lib.rs indexed");
    assert!(lib_rs.contains("a_changed"), "lib.rs must be published");
    let second_rs = content_of(&connection, "second.rs").expect("second.rs indexed");
    assert!(
        second_rs.contains("second_v2"),
        "the in-publish change must be reconciled too"
    );
    let revisions: i64 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .expect("count revisions");
    assert_eq!(
        revisions, 3,
        "the baseline plus one revision per publishing barrier iteration"
    );
}

/// The post-verification drain is a second failure surface: the full
/// publish can succeed and the events that arrived while it ran can still
/// fail to reconcile. The exact failure must reach the caller instead of
/// being swallowed as a healthy result.
#[test]
fn a_failed_post_verification_drain_surfaces_the_exact_error() {
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let manager1 = SourceSyncManager::with_watcher();
    sync_project(&manager1, &project);
    drop(manager1);

    let manager2 = SourceSyncManager::with_watcher();
    let state = manager2.activate(project.path());
    // Abort only the reconcile's insert. The full publish below has
    // nothing to publish (the disk state still matches the stored
    // revision), so it never touches `files`; the drain does.
    {
        let connection = open_db(&project);
        connection
            .execute_batch(
                "CREATE TRIGGER abort_drain_insert BEFORE INSERT ON files \
                 BEGIN SELECT RAISE(ABORT, 'injected drain failure'); END;",
            )
            .expect("install the failing trigger");
    }

    // Change a file after the publish sampled it: the publish still sees
    // the old content, so its diff is empty and it commits nothing; the
    // event lands in the post-verification drain, which then fails.
    let hook_root = project.path().to_path_buf();
    let hook_state = state.clone();
    crate::sync::race_hook::set(project.path(), move || {
        fs::write(hook_root.join("lib.rs"), b"pub fn drained_change() {}\n")
            .expect("write during the full publish");
        hook_state.mark_dirty("lib.rs".to_owned());
    });

    let result = manager2.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    let error = match result {
        Ok(_) => panic!("a failed post-verification drain must surface an error"),
        Err(error) => error,
    };
    assert_eq!(
        error.message,
        "draining events after verification: reconcile error: database error: \
         injected drain failure",
        "the exact drain failure must reach the caller"
    );
    assert!(
        error.data.is_none(),
        "a database failure is not a coded budget condition"
    );
}

/// The corrupt-open recovery must not swallow a failure to discard the
/// broken database: the exact discard error reaches the caller, and no
/// half-recovered database is published into.
#[test]
fn a_failure_to_discard_a_corrupt_database_surfaces_the_exact_discard_error() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);

    let database = project.path().join(".planning/slugaudit/project.db");
    let wal = std::path::PathBuf::from(format!("{}-wal", database.display()));
    let shm = std::path::PathBuf::from(format!("{}-shm", database.display()));
    remove_if_present(&wal);
    remove_if_present(&shm);
    // A directory at the sidecar path lets SQLite read the main file
    // first and report it as not-a-database (the corruption branch),
    // while the recovery's sidecar removal then fails with EISDIR.
    fs::create_dir(&shm).expect("make the shm sidecar a directory");
    fs::write(&database, b"this is not a sqlite database").expect("corrupt the database");

    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    let error = match result {
        Ok(_) => panic!("a failed discard must surface an error"),
        Err(error) => error,
    };

    let discard_error = fs::remove_file(&shm).expect_err("removing a directory sidecar must fail");
    assert_eq!(
        error.message,
        format!("discarding the corrupt or outdated project database: {discard_error}"),
        "the exact discard failure must reach the caller"
    );
    assert!(
        error.data.is_none(),
        "a discard failure is not a coded, recoverable condition"
    );
}

/// A metadata failure that is neither a stale root nor an incompatible
/// version is surfaced unchanged (no discard/rebuild), with its exact
/// message — an operator must see why the database could not be verified.
#[test]
fn a_non_stale_metadata_failure_is_surfaced_unchanged() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    let synced = sync_project(&manager, &project);

    {
        let connection = crate::store::open_read_write(&synced.database_path).expect("open db");
        connection
            .execute_batch("ALTER TABLE project RENAME TO project_backing")
            .expect("hide the project metadata table");
    }

    let result = manager.ensure_current(
        &project.path().to_string_lossy(),
        &crate::progress::NoopProgressSink,
    );
    let error = match result {
        Ok(_) => panic!("an unreadable metadata table must surface an error"),
        Err(error) => error,
    };
    assert_eq!(
        error.message, "recording project metadata: no such table: project",
        "a non-stale metadata failure must surface unchanged"
    );
    assert!(
        error.data.is_none(),
        "the untouched metadata failure carries no code"
    );
}
