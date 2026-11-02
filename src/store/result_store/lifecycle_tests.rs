use super::harness_tests::*;
use super::*;
use crate::store::test_join::join_scoped_checked;
use slugaudit_test_check::{Check, TestResult};
use std::path::Path;
use uuid::Uuid;

#[cfg(target_os = "linux")]
fn assert_no_leftovers(fx: &Fixture) -> TestResult {
    assert!(db_files(&fx.results_dir())?.is_empty());
    assert_eq!(
        deleted_descriptors_under(fx.dir.path())?,
        Vec::<String>::new()
    );
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .admitted_bytes,
        0
    );
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_an_over_budget_write_aborts_itself_and_leaves_no_file_or_descriptor() -> TestResult
{
    let fx = fixture(64 * 1024)?;

    let outcome = write_rows(&fx.store, 2_500, 100);

    assert!(
        matches!(outcome, Err(ResultStoreError::TooLarge)),
        "{outcome:?}"
    );
    assert_no_leftovers(&fx)?;
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .refused_too_large,
        1
    );
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_a_failure_after_the_first_batch_leaves_nothing_and_the_next_write_works()
-> TestResult {
    let fx = fixture(1 << 30)?;
    let mut writer = fx.store.begin(spec(&["id", "payload"])).check("begin")?;
    writer
        .inject(super::fault::Fault::FailBeforeBatch(2))
        .check("arm")?;
    let mut outcome = Ok(());
    for index in 0..3_000_i64 {
        outcome = writer.push(&[Cell::Int(index), Cell::Text("x".to_owned())]);
        if outcome.is_err() {
            break;
        }
    }
    drop(writer);

    assert!(
        matches!(outcome, Err(ResultStoreError::Unavailable { .. })),
        "{outcome:?}"
    );
    assert_no_leftovers(&fx)?;

    let next = write_rows(&fx.store, 1_500, 5).check("the store still works")?;
    assert_eq!(read_all(&fx.store, next.id)?.len(), 1_500);
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_a_full_disk_is_unavailable_and_keeps_the_sqlite_source() -> TestResult {
    let fx = fixture(1 << 30)?;
    let mut writer = fx.store.begin(spec(&["payload"])).check("begin")?;
    writer
        .inject(super::fault::Fault::MaxPages(8))
        .check("arm")?;
    let payload = Cell::Text("y".repeat(1_000));
    let mut failure = None;
    for _ in 0..500 {
        if let Err(error) = writer.push(std::slice::from_ref(&payload)) {
            failure = Some(error);
            break;
        }
    }
    drop(writer);

    let Some(ResultStoreError::Unavailable { source, .. }) = failure else {
        return Err(format!("expected Unavailable, got {failure:?}")).check("unexpected outcome");
    };
    let sqlite = source
        .downcast_ref::<rusqlite::Error>()
        .check("sqlite source")?;
    assert!(
        matches!(sqlite, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::DiskFull),
        "{sqlite:?}"
    );
    assert_no_leftovers(&fx)?;
    Ok(())
}

const CHILD_DIR_ENV: &str = "SLUGAUDIT_RESULT_STORE_ABORT_DIR";

/// Runs only when the parent arms it through the environment; then it opens a
/// store, writes more than one batch, and dies without cleanup.
#[test]
fn result_store_abort_helper_child_is_a_no_op_unless_the_parent_arms_it() -> TestResult {
    let Some(dir) = std::env::var_os(CHILD_DIR_ENV) else {
        return Ok(());
    };
    let store =
        ResultStore::open(Path::new(&dir), Uuid::new_v4(), limits(1 << 30)).check("open")?;
    let mut writer = store.begin(spec(&["id"])).check("begin")?;
    for index in 0..2_500_i64 {
        writer.push(&[Cell::Int(index)]).check("push")?;
    }
    std::process::abort();
}

#[test]
#[cfg(unix)]
fn result_store_a_process_abort_mid_write_leaves_no_result_file() -> TestResult {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().check("temp dir")?;

    let status = std::process::Command::new(std::env::current_exe().check("test binary")?)
        .args([
            "--exact",
            "store::result_store::lifecycle_tests::result_store_abort_helper_child_is_a_no_op_unless_the_parent_arms_it",
            "--test-threads=1",
        ])
        .env(CHILD_DIR_ENV, dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .check("spawn the helper")?;

    assert_eq!(
        status.signal(),
        Some(6),
        "the helper must die of SIGABRT: {status:?}"
    );
    assert!(db_files(&dir.path().join("results"))?.is_empty());
    Ok(())
}

fn plant(dir: &Path, name: &str) -> TestResult {
    std::fs::create_dir_all(dir.join("results")).check("results dir")?;
    std::fs::write(dir.join("results").join(name), b"x").check("plant")?;
    Ok(())
}

fn db_name(session: Uuid) -> String {
    format!(
        "{}-{}.db",
        session.hyphenated(),
        Uuid::new_v4().hyphenated()
    )
}

fn open_in(dir: &Path, session: Uuid) -> TestResult<ResultStore> {
    ResultStore::open(dir, session, limits(1 << 30)).check("open")
}

fn listing(dir: &Path) -> TestResult<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir.join("results")).check("list")? {
        names.push(
            entry
                .check("entry")?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    names.sort();
    Ok(names)
}

#[test]
fn result_store_open_sweeps_the_files_and_lock_of_a_dead_owner() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let dead = Uuid::new_v4();
    plant(dir.path(), &db_name(dead))?;
    plant(dir.path(), &db_name(dead))?;
    plant(dir.path(), &format!("{}.lock", dead.hyphenated()))?;

    let session = Uuid::new_v4();
    let store = open_in(dir.path(), session)?;

    assert_eq!(
        listing(dir.path())?,
        vec![format!("{}.lock", session.hyphenated())]
    );
    assert_eq!(
        store
            .stats()
            .check("read the store counters")?
            .sweep
            .removed_files,
        3
    );
    Ok(())
}

#[test]
fn result_store_open_leaves_the_files_of_a_live_owner() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let live_session = Uuid::new_v4();
    let live = open_in(dir.path(), live_session)?;
    let in_flight = db_name(live_session);
    plant(dir.path(), &in_flight)?;

    let second = open_in(dir.path(), Uuid::new_v4())?;

    assert!(listing(dir.path())?.contains(&in_flight));
    assert_eq!(
        second
            .stats()
            .check("read the store counters")?
            .sweep
            .live_owners_skipped,
        1
    );
    drop(live);
    Ok(())
}

#[test]
fn result_store_open_sweeps_a_result_file_with_no_lock_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let orphan = db_name(Uuid::new_v4());
    plant(dir.path(), &orphan)?;

    let store = open_in(dir.path(), Uuid::new_v4())?;

    assert!(!listing(dir.path())?.contains(&orphan));
    assert_eq!(
        store
            .stats()
            .check("read the store counters")?
            .sweep
            .removed_files,
        1
    );
    Ok(())
}

#[test]
fn result_store_open_never_sweeps_its_own_sessions_files() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let session = Uuid::new_v4();
    let own = db_name(session);
    plant(dir.path(), &own)?;

    let store = open_in(dir.path(), session)?;

    assert!(listing(dir.path())?.contains(&own));
    assert_eq!(
        store
            .stats()
            .check("read the store counters")?
            .sweep
            .removed_files,
        0
    );
    Ok(())
}

#[test]
fn result_store_open_leaves_files_that_are_not_ours() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    plant(dir.path(), "notes.db")?;
    plant(dir.path(), "readme.txt")?;

    open_in(dir.path(), Uuid::new_v4())?;

    let names = listing(dir.path())?;
    assert!(names.contains(&"notes.db".to_owned()) && names.contains(&"readme.txt".to_owned()));
    Ok(())
}

#[test]
fn result_store_a_sweep_removal_failure_is_counted_and_keeps_the_first_error() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let not_a_file = dir.path().join("a-directory");
    std::fs::create_dir(&not_a_file).check("dir")?;
    let mut outcome = SweepOutcome::default();

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        outcome.remove(&not_a_file, "remove the zz-sweep-probe file");
    })?;

    assert_eq!(outcome.errors, 1);
    assert!(
        outcome
            .first_error
            .as_deref()
            .is_some_and(|e| e.starts_with("remove the zz-sweep-probe file"))
    );
    assert_eq!(
        logs.lines()
            .filter(|l| l.contains("zz-sweep-probe"))
            .count(),
        1,
        "{logs}"
    );
    Ok(())
}

#[test]
fn result_store_a_second_open_of_the_same_session_is_unavailable() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let session = Uuid::new_v4();
    let first = open_in(dir.path(), session)?;

    let second = ResultStore::open(dir.path(), session, limits(1 << 30));

    assert!(
        matches!(second, Err(ResultStoreError::Unavailable { .. })),
        "{:?}",
        second.err()
    );
    drop(first);
    Ok(())
}

#[test]
#[cfg(unix)]
fn result_store_open_refuses_a_symlinked_results_directory() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let elsewhere = tempfile::tempdir().check("elsewhere")?;
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("results")).check("symlink")?;

    let outcome = ResultStore::open(dir.path(), Uuid::new_v4(), limits(1 << 30));

    assert!(
        matches!(outcome, Err(ResultStoreError::Unavailable { .. })),
        "{:?}",
        outcome.err()
    );
    assert_eq!(
        std::fs::read_dir(elsewhere.path()).check("list")?.count(),
        0
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn result_store_the_directory_is_private_and_the_lock_file_is_0600() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture(1 << 30)?;

    let dir_mode = std::fs::metadata(fx.results_dir())
        .check("dir")?
        .permissions()
        .mode()
        & 0o777;
    let lock = std::fs::read_dir(fx.results_dir())
        .check("list")?
        .next()
        .check("lock")?
        .check("entry")?;
    let lock_mode = lock.metadata().check("meta")?.permissions().mode() & 0o777;

    assert_eq!((dir_mode, lock_mode), (0o700, 0o600));
    Ok(())
}

#[test]
fn result_store_dropping_the_store_removes_its_lock_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let store = open_in(dir.path(), Uuid::new_v4())?;
    assert_eq!(listing(dir.path())?.len(), 1);

    drop(store);

    assert!(listing(dir.path())?.is_empty());
    Ok(())
}

/// A deadlock in the store must fail the test, not hang the suite.
const THREADS_DEADLINE: std::time::Duration = std::time::Duration::from_mins(2);

/// Seeds, then runs writers and pagers together; returns the seeded and
/// the concurrently written handles.
fn run_threads(fx: &Fixture) -> TestResult<(Vec<ResultHandle>, Vec<Vec<ResultHandle>>)> {
    let seeded = (0..4)
        .map(|_| write_rows(&fx.store, 500, 20).check("seed"))
        .collect::<TestResult<Vec<ResultHandle>>>()?;
    let store = &fx.store;
    let written = std::thread::scope(|scope| -> TestResult<Vec<Vec<ResultHandle>>> {
        let writers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(move || {
                    (0..3)
                        .map(|_| write_rows(store, 1_200, 20).check("concurrent write"))
                        .collect::<TestResult<Vec<_>>>()
                })
            })
            .collect();
        let pagers: Vec<_> = seeded
            .iter()
            .map(|handle| {
                let id = handle.id;
                scope.spawn(move || -> TestResult {
                    for _ in 0..5 {
                        assert_eq!(read_all(store, id)?.len(), 500);
                    }
                    Ok(())
                })
            })
            .collect();
        for pager in pagers {
            join_scoped_checked(pager, "pager thread")??;
        }
        writers
            .into_iter()
            .map(|writer| join_scoped_checked(writer, "writer thread").and_then(|inner| inner))
            .collect()
    })?;
    Ok((seeded, written))
}

#[test]
fn result_store_writers_and_pagers_on_many_threads_finish_with_consistent_totals() -> TestResult {
    let fx = std::sync::Arc::new(fixture(1 << 30)?);
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::sync::Arc::clone(&fx);
    std::thread::spawn(move || {
        // The receiver is gone only after the deadline already failed the test.
        if sender.send(run_threads(&worker)).is_err() {
            tracing::warn!("the thread test gave up before its workers finished");
        }
    });

    let (seeded, written) = receiver
        .recv_timeout(THREADS_DEADLINE)
        .check("writers and pagers finished before the deadline (a deadlock or a panic)")??;

    let all: Vec<&ResultHandle> = seeded.iter().chain(written.iter().flatten()).collect();
    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!(all.len(), 16);
    assert_eq!(stats.active.results + stats.delivered.results, 16);
    assert_eq!(
        stats.admitted_bytes,
        all.iter().map(|h| h.bytes).sum::<u64>()
    );
    for handle in written.iter().flatten() {
        assert_eq!(read_all(&fx.store, handle.id)?.len(), 1_200);
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn result_store_open_refuses_a_symlinked_lock_file() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let elsewhere = tempfile::tempdir().check("elsewhere")?;
    let target = elsewhere.path().join("target");
    std::fs::write(&target, b"keep").check("target")?;
    let session = Uuid::new_v4();
    let results = dir.path().join("results");
    std::fs::create_dir(&results).check("results dir")?;
    std::os::unix::fs::symlink(
        &target,
        results.join(format!("{}.lock", session.hyphenated())),
    )
    .check("symlink")?;

    let outcome = ResultStore::open(dir.path(), session, limits(1 << 30));

    assert!(
        matches!(outcome, Err(ResultStoreError::Unavailable { .. })),
        "{:?}",
        outcome.err()
    );
    assert_eq!(std::fs::read(&target).check("target intact")?, b"keep");
    Ok(())
}
