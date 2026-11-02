//! Shared fixtures for the result store tests: a store in a temp directory
//! with an injected clock, row builders, and directory/descriptor inspection.

use super::*;
use slugaudit_test_check::{Check, TestResult};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use uuid::Uuid;

pub(super) const GRACE_SECS: u64 = 900;
/// [`GRACE_SECS`] as the signed clock value the tests add to timestamps.
pub(super) const GRACE_SECS_SIGNED: i64 = GRACE_SECS.cast_signed();

pub(super) struct Fixture {
    pub(super) dir: tempfile::TempDir,
    pub(super) time: Arc<AtomicI64>,
    pub(super) store: ResultStore,
}

impl Fixture {
    pub(super) fn set_time(&self, seconds: i64) {
        self.time.store(seconds, Ordering::SeqCst);
    }

    pub(super) fn results_dir(&self) -> PathBuf {
        self.dir.path().join("results")
    }
}

pub(super) fn clock_at(time: &Arc<AtomicI64>) -> Clock {
    let time = Arc::clone(time);
    Arc::new(move || Ok(time.load(Ordering::SeqCst)))
}

pub(super) fn limits(max_bytes: u64) -> ResultStoreLimits {
    ResultStoreLimits {
        max_bytes,
        active_grace_secs: GRACE_SECS,
    }
}

pub(super) fn fixture(max_bytes: u64) -> TestResult<Fixture> {
    let dir = tempfile::tempdir().check("temp dir")?;
    let time = Arc::new(AtomicI64::new(1_000));
    let store = ResultStore::open_with_clock(
        dir.path(),
        Uuid::new_v4(),
        limits(max_bytes),
        clock_at(&time),
    )
    .check("open the store")?;
    Ok(Fixture { dir, time, store })
}

pub(super) fn spec(columns: &[&str]) -> ResultSpec {
    ResultSpec {
        tool: "query".to_owned(),
        project: "proj".to_owned(),
        revision_id: "rev-1".to_owned(),
        columns: columns.iter().map(|name| (*name).to_owned()).collect(),
    }
}

pub(super) fn complete() -> serde_json::Value {
    serde_json::json!({"kind": "complete"})
}

/// `rows` rows of `[Int(i), Text(payload_len x 'x')]`.
pub(super) fn write_rows(
    store: &ResultStore,
    rows: usize,
    payload_len: usize,
) -> Result<ResultHandle, ResultStoreError> {
    let mut writer = store.begin(spec(&["id", "payload"]))?;
    let payload = "x".repeat(payload_len);
    for (index, _) in (0_i64..).zip(0..rows) {
        writer.push(&[Cell::Int(index), Cell::Text(payload.clone())])?;
    }
    writer.finish(&complete())
}

/// Reads one row and stops, leaving the result unfinished.
pub(super) fn touch(store: &ResultStore, id: ResultId) -> TestResult<PageRead> {
    store.read_page(id, 0, |_| Take::No).check("read one row")
}

/// Reads to the end, returning the ordinal of every row.
pub(super) fn read_all(store: &ResultStore, id: ResultId) -> TestResult<Vec<StoredRow>> {
    Ok(store
        .read_page(id, 0, |_| Take::Yes)
        .check("read all rows")?
        .rows)
}

/// Reads the page a token names: the production path, decode then read.
pub(super) fn read_token(store: &ResultStore, token: &str) -> Result<PageRead, ResultStoreError> {
    let cursor = store.decode_cursor(token)?;
    store.read_page(cursor.result, cursor.ord, |_| Take::Yes)
}

pub(super) fn db_files(results_dir: &std::path::Path) -> TestResult<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(results_dir).check("list results")? {
        let name = entry
            .check("entry")?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if std::path::Path::new(&name)
            .extension()
            .is_some_and(|extension| extension == "db")
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Descriptors of this process that point at a deleted file under `dir`.
#[cfg(target_os = "linux")]
pub(super) fn deleted_descriptors_under(dir: &std::path::Path) -> TestResult<Vec<String>> {
    let root = dir.canonicalize().check("canonical dir")?;
    let root = root.to_string_lossy();
    let mut deleted = Vec::new();
    for entry in std::fs::read_dir("/proc/self/fd").check("list descriptors")? {
        let entry = entry.check("fd entry")?;
        let target = match std::fs::read_link(entry.path()) {
            Ok(target) => target.to_string_lossy().into_owned(),
            // The listing's own descriptor is already closed: not a leak.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).check("read a descriptor link"),
        };
        if target.starts_with(root.as_ref()) && target.ends_with("(deleted)") {
            deleted.push(target);
        }
    }
    Ok(deleted)
}
