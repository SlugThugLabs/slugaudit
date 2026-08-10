//! Search benchmarks: representative `query`-tool workloads run against a
//! fully synced project database. Search is a bounded-scan over the
//! `files.content` / `evidence.payload` columns (SQLite `LIKE`) — there is
//! no FTS5 table yet, so these numbers are the honest baseline for the
//! §21.4 "FTS5 versus bounded-scan search" decision. Also included is the
//! recursive-CTE dependency traversal over `dependency_edges`, which the
//! `query` tool exposes for dependents/dependencies lookups.
//!
//! The fixture is synced once (outside the timed loop) via the real
//! `SourceSyncManager` path, then the read-only connection used by the
//! `query` tool is benchmarked directly.

mod common;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::project::{ProjectRoot, database_path};
use slugaudit_mcp_rust::store;
use slugaudit_mcp_rust::sync::SourceSyncManager;
use std::path::PathBuf;

/// A synced fixture whose temp dir is kept alive for the whole group.
struct SyncedDb {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn build_synced(file_count: usize) -> SyncedDb {
    let dir = tempfile::tempdir().expect("temp dir");
    let _ = common::generate_fixture(dir.path(), file_count);
    let root = dir.path().to_str().expect("utf-8 fixture path").to_owned();
    SourceSyncManager::new()
        .ensure_current(&root, &NoopProgressSink)
        .expect("initial sync");
    let root = ProjectRoot::resolve(dir.path()).expect("valid project root");
    SyncedDb {
        _dir: dir,
        path: database_path(&root),
    }
}

/// Runs `sql` and returns the first column of the first row as a count.
fn run_count(connection: &rusqlite::Connection, sql: &str) -> i64 {
    let mut statement = connection.prepare(sql).expect("prepare search SQL");
    let mut rows = statement.query([]).expect("execute search SQL");
    let row = rows.next().expect("one result row").expect("read row");
    black_box(row.get(0).expect("count column"))
}

/// Substring search over full file content (the common search workload).
const SUBSTRING_SQL: &str = "SELECT COUNT(*) FROM files WHERE content LIKE '%shared_helper%'";
/// Symbol lookup: filter evidence by kind and by payload content.
const SYMBOL_SQL: &str = "SELECT COUNT(*) FROM evidence e JOIN files f ON f.id = e.file_id \
     WHERE e.kind = 'Symbol' AND e.payload LIKE '%needle_17%'";
/// Dependency traversal: recursive CTE over dependency_edges from one Rust
/// module (the `query` tool's dependents/dependencies pattern).
const TRAVERSAL_SQL: &str = "WITH RECURSIVE reach(from_id) AS ( \
       SELECT id FROM files WHERE path = 'src/rust/mod_1.rs' \
       UNION \
       SELECT e.to_file_id FROM dependency_edges e JOIN reach r ON e.from_file_id = r.from_id \
       WHERE e.to_file_id IS NOT NULL \
     ) SELECT COUNT(*) FROM reach";

fn bench_search(c: &mut Criterion) {
    let small = build_synced(common::SMALL);
    let large = build_synced(common::LARGE);
    let small_connection = store::open_read_only(&small.path).expect("open read-only");
    let large_connection = store::open_read_only(&large.path).expect("open read-only");

    let mut group = c.benchmark_group("search");
    group.sample_size(20);

    group.bench_function("substring_like_small", |b| {
        b.iter(|| run_count(&small_connection, SUBSTRING_SQL));
    });
    group.bench_function("substring_like_large", |b| {
        b.iter(|| run_count(&large_connection, SUBSTRING_SQL));
    });
    group.bench_function("symbol_lookup_large", |b| {
        b.iter(|| run_count(&large_connection, SYMBOL_SQL));
    });
    group.bench_function("dependency_traversal_large", |b| {
        b.iter(|| run_count(&large_connection, TRAVERSAL_SQL));
    });

    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
