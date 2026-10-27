//! Shared fixtures for the result store tests: a store in a temp directory
//! with an injected clock, row builders, and directory/descriptor inspection.

use super::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use uuid::Uuid;

pub(super) const GRACE_SECS: u64 = 900;

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
    Arc::new(move || time.load(Ordering::SeqCst))
}

pub(super) fn limits(max_bytes: u64) -> ResultStoreLimits {
    ResultStoreLimits {
        max_bytes,
        active_grace_secs: GRACE_SECS,
    }
}

pub(super) fn fixture(max_bytes: u64) -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let time = Arc::new(AtomicI64::new(1_000));
    let store = ResultStore::open_with_clock(
        dir.path(),
        Uuid::new_v4(),
        limits(max_bytes),
        clock_at(&time),
    )
    .expect("open the store");
    Fixture { dir, time, store }
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
    for index in 0..rows {
        writer.push(&[Cell::Int(index as i64), Cell::Text(payload.clone())])?;
    }
    writer.finish(&complete())
}

/// Reads one row and stops, leaving the result unfinished.
pub(super) fn touch(store: &ResultStore, id: ResultId) -> PageRead {
    store.read_page(id, 0, |_| Take::No).expect("read one row")
}

/// Reads to the end, returning the ordinal of every row.
pub(super) fn read_all(store: &ResultStore, id: ResultId) -> Vec<StoredRow> {
    store
        .read_page(id, 0, |_| Take::Yes)
        .expect("read all rows")
        .rows
}

pub(super) fn db_files(results_dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(results_dir)
        .expect("list results")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".db"))
        .collect();
    names.sort();
    names
}

/// Descriptors of this process that point at a deleted file under `dir`.
#[cfg(target_os = "linux")]
pub(super) fn deleted_descriptors_under(dir: &std::path::Path) -> Vec<String> {
    let root = dir.canonicalize().expect("canonical dir");
    std::fs::read_dir("/proc/self/fd")
        .expect("list descriptors")
        .filter_map(|entry| std::fs::read_link(entry.expect("fd entry").path()).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .filter(|target| {
            target.starts_with(root.to_string_lossy().as_ref()) && target.ends_with("(deleted)")
        })
        .collect()
}
