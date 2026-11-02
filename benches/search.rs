#![forbid(unsafe_code)]

//! Search benchmarks: representative `query`-tool workloads run against a
//! fully synced project database. Search is a bounded-scan over the
//! `files.content` / `evidence.payload` columns (SQLite `LIKE`) — there is
//! no FTS5 table yet, so these numbers are the honest baseline for the
//! "FTS5 versus bounded-scan search" decision. Also included is the
//! recursive-CTE dependency traversal over `dependency_edges`, which the
//! `query` tool exposes for dependents/dependencies lookups.
//!
//! The fixture is synced once (outside the timed loop) via the real
//! `SourceSyncManager` path, then the read-only connection used by the
//! `query` tool is benchmarked directly.

mod common;

use common::failure::{FirstFailure, exit_code};

/// Small fixture size: 40 files (the cheap contrast to `common::LARGE`).
const SMALL: usize = 40;

use criterion::Criterion;
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::project::{ProjectRoot, database_path};
use slugaudit_mcp_rust::store;
use slugaudit_mcp_rust::sync::SourceSyncManager;
use slugaudit_test_check::{Check, TestResult};
use std::hint::black_box;
use std::path::PathBuf;
use std::process::ExitCode;

/// A synced fixture whose temp dir is kept alive for the whole group.
struct SyncedDb {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn build_synced(file_count: usize) -> TestResult<SyncedDb> {
    let dir = tempfile::tempdir().check("create fixture dir")?;
    common::generate_fixture(dir.path(), file_count)?.log("search");
    let root = dir
        .path()
        .to_str()
        .check("fixture path is utf-8")?
        .to_owned();
    SourceSyncManager::new()
        .ensure_current(&root, &NoopProgressSink)
        .check("initial sync")?;
    let root = ProjectRoot::resolve(dir.path()).check("resolve project root")?;
    Ok(SyncedDb {
        path: database_path(&root),
        _dir: dir,
    })
}

/// Runs `sql` and returns the first column of the first row as a count.
fn run_count(connection: &rusqlite::Connection, sql: &str) -> TestResult<i64> {
    let mut statement = connection.prepare(sql).check("prepare search SQL")?;
    let mut rows = statement.query([]).check("execute search SQL")?;
    let row = rows
        .next()
        .check("read the result row")?
        .check("query returned one row")?;
    let count: i64 = row.get(0).check("read count column")?;
    Ok(black_box(count))
}

/// Substring search over full file content (the common search workload).
const SUBSTRING_SQL: &str = "SELECT COUNT(*) FROM files WHERE content LIKE '%shared_helper%'";
/// Symbol lookup: filter evidence by kind and by payload content.
const SYMBOL_SQL: &str = "SELECT COUNT(*) FROM evidence e JOIN files f ON f.id = e.file_id \
     WHERE e.kind = 'Symbol' AND e.payload LIKE '%needle_17%'";
/// Dependency traversal: recursive CTE over `dependency_edges` from one Rust
/// module (the `query` tool's dependents/dependencies pattern).
const TRAVERSAL_SQL: &str = "WITH RECURSIVE reach(from_id) AS ( \
       SELECT id FROM files WHERE path = 'src/rust/mod_1.rs' \
       UNION \
       SELECT e.to_file_id FROM dependency_edges e JOIN reach r ON e.from_file_id = r.from_id \
       WHERE e.to_file_id IS NOT NULL \
     ) SELECT COUNT(*) FROM reach";

fn bench_search(c: &mut Criterion) -> TestResult {
    let small = build_synced(SMALL)?;
    let large = build_synced(common::LARGE)?;
    let small_connection = store::open_existing(&small.path).check("open small database")?;
    let large_connection = store::open_existing(&large.path).check("open large database")?;
    let failure = FirstFailure::new();

    let mut group = c.benchmark_group("search");
    group.sample_size(15);

    group.bench_function("substring_like_small", |b| {
        b.iter(|| failure.record(run_count(&small_connection, SUBSTRING_SQL)));
    });
    group.bench_function("substring_like_large", |b| {
        b.iter(|| failure.record(run_count(&large_connection, SUBSTRING_SQL)));
    });
    group.bench_function("symbol_lookup_large", |b| {
        b.iter(|| failure.record(run_count(&large_connection, SYMBOL_SQL)));
    });
    group.bench_function("dependency_traversal_large", |b| {
        b.iter(|| failure.record(run_count(&large_connection, TRAVERSAL_SQL)));
    });

    group.finish();
    failure.finish()
}

fn main() -> ExitCode {
    let mut criterion = Criterion::default().configure_from_args();
    let result = bench_search(&mut criterion);
    criterion.final_summary();
    exit_code(result)
}
