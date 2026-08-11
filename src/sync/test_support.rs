//! Shared fixture helpers for `sync` tests. Previously each test file
//! carried its own copy (`write`: 8 copies, `stored_paths`: 3 copies), so
//! a change to how fixture files are written or queried now lives in
//! exactly one place.
use std::fs;
use std::path::Path;

use rusqlite::Connection;

/// Writes `relative` under `root`, creating parent directories on demand.
pub(crate) fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

/// Reads every stored path in the `files` table, sorted.
pub(crate) fn stored_paths(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .expect("prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}
