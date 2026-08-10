//! Sync benchmarks through the real `SourceSyncManager::ensure_current`
//! path: first sync on a fresh database, unchanged sync (the manifest
//! matches, so nothing is re-written), and changed-file sync (one file
//! modified per iteration). Also reports SQLite database size growth.
//!
//! The manager is created with `SourceSyncManager::new()` — no filesystem
//! watcher — so every `ensure_current` runs the full-verification publish
//! path (`WatcherHealth::Unavailable`). These numbers are therefore the
//! *worst case* for unchanged and changed sync: a watcher-trusted
//! deployment skips the walk-and-hash entirely when the watcher is healthy,
//! which is measured by functional tests rather than here. See
//! `.planning/PERFORMANCE.md` for the note.

mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::sync::SourceSyncManager;
use std::path::Path;
use std::time::Duration;

fn reset_database(root: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let path = root
            .join(".planning")
            .join("slugaudit")
            .join(format!("project.db{suffix}"));
        let _ = std::fs::remove_file(path);
    }
}

fn database_size(root: &Path) -> u64 {
    std::fs::metadata(root.join(".planning/slugaudit/project.db"))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn bench_sync(c: &mut Criterion) {
    for file_count in [common::SMALL, common::LARGE] {
        let dir = tempfile::tempdir().expect("temp dir");
        let stats = common::generate_fixture(dir.path(), file_count);
        let root = dir.path().to_path_buf();
        let root_str = root.to_str().expect("utf-8 fixture path").to_owned();
        let manager = SourceSyncManager::new();

        let mut group = c.benchmark_group(format!("sync_{file_count}"));
        // First sync is expensive (it parses every file); keep the sample
        // count low so the whole suite stays within a few minutes.
        group
            .sample_size(10)
            .measurement_time(Duration::from_secs(8));

        group.bench_function("first_sync", |b| {
            b.iter(|| {
                reset_database(&root);
                manager
                    .ensure_current(&root_str, &NoopProgressSink)
                    .expect("first sync");
            });
        });
        group.finish();
        eprintln!(
            "db_size_after_first_sync_{file_count}: {} bytes",
            database_size(&root)
        );

        // Warm once, then measure steady-state unchanged sync. With the
        // watcher unavailable this re-walks and re-hashes the whole tree
        // but publishes nothing (manifest unchanged) — the worst case.
        manager
            .ensure_current(&root_str, &NoopProgressSink)
            .expect("warm-up sync");
        let mut group = c.benchmark_group(format!("sync_{file_count}"));
        group
            .sample_size(10)
            .measurement_time(Duration::from_secs(8));
        group.bench_function("unchanged_sync", |b| {
            b.iter(|| {
                manager
                    .ensure_current(&root_str, &NoopProgressSink)
                    .expect("unchanged sync");
            });
        });
        group.finish();
        eprintln!(
            "db_size_after_unchanged_sync_{file_count}: {} bytes",
            database_size(&root)
        );

        // One file modified per iteration; each iteration re-parses exactly
        // the touched file and publishes the delta.
        let mut touched = 0_usize;
        let mut group = c.benchmark_group(format!("sync_{file_count}"));
        group
            .sample_size(10)
            .measurement_time(Duration::from_secs(8));
        group.bench_function("changed_file_sync", |b| {
            b.iter(|| {
                common::touch_rust_file(&root, touched % stats.rust_count, touched);
                touched += 1;
                manager
                    .ensure_current(&root_str, &NoopProgressSink)
                    .expect("changed-file sync");
            });
        });
        group.finish();
        eprintln!(
            "db_size_after_changed_sync_{file_count}: {} bytes",
            database_size(&root)
        );
    }
}

criterion_group!(benches, bench_sync);
criterion_main!(benches);
