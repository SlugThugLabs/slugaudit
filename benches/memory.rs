#![forbid(unsafe_code)]

//! Peak-memory benchmark.
//!
//! This bench records peak RSS for the "Memory, 200-file fixture" budget. It
//! runs the full `SourceSyncManager` sync
//! pipeline against the standard `LARGE` (200-file) fixture and reports
//! the peak resident-set size measured via `/proc/self/status`'s
//! `VmHWM` (high-water-mark RSS) on Linux.
//!
//! The bench measures the in-process peak (this bencher process), which is
//! the realistic peak a single MCP tool call pays — no swap, no OS
//! allocator slot pressure specific to a child process. On non-Linux it
//! reports no figure; the bench still runs and the number is declared as
//! "unmeasured on this OS" rather than skipped, so cross-machine
//! comparison is unambiguous.
mod common;

use common::failure::{FirstFailure, exit_code};
use criterion::Criterion;
use slugaudit_test_check::{Check, TestResult};
#[cfg(target_os = "linux")]
use std::io::BufRead;
use std::process::ExitCode;

/// Linux-only: peak resident-set size in kibibytes, read from
/// `/proc/self/status`'s `VmHWM` field. `Err` carries the real cause when
/// the file is unreadable or the field is missing or malformed.
#[cfg(target_os = "linux")]
fn peak_rss_kib() -> Result<Option<u64>, String> {
    let file = std::fs::File::open("/proc/self/status")
        .map_err(|err| format!("cannot open /proc/self/status: {err}"))?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_err(|err| format!("cannot read /proc/self/status: {err}"))?;
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            // "  12345 kB" — split on whitespace, take the first numeric token.
            let token = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| format!("VmHWM line has no value: {line:?}"))?;
            let kib = token
                .parse()
                .map_err(|err| format!("VmHWM value {token:?} is not a number: {err}"))?;
            return Ok(Some(kib));
        }
    }
    Err("/proc/self/status has no VmHWM line".to_owned())
}

/// Peak RSS is only read on Linux; elsewhere the figure is unmeasured.
#[cfg(not(target_os = "linux"))]
fn peak_rss_kib() -> Result<Option<u64>, String> {
    Ok(None)
}

/// One timed iteration: a fresh database, project activation, then a full
/// sync. Returns the sync's wall-clock time.
fn timed_full_sync(project: &std::path::Path) -> TestResult<std::time::Duration> {
    // Fresh database each iteration so the WAL doesn't grow
    // between samples (which would inflate RSS).
    let db_dir = tempfile::tempdir().check("create database dir")?;
    let database_path = db_dir.path().join("project.db");
    let connection =
        slugaudit_mcp_rust::store::open_read_write(&database_path).check("open database")?;

    let project_root =
        slugaudit_mcp_rust::project::ProjectRoot::resolve(project).check("resolve project root")?;
    slugaudit_mcp_rust::project::enable(&project_root).check("activate project")?;

    let start = std::time::Instant::now();
    let synced = slugaudit_mcp_rust::sync::SourceSyncManager::new()
        .ensure_current(
            &project.to_string_lossy(),
            &slugaudit_mcp_rust::progress::NoopProgressSink,
        )
        .check("sync the fixture")?;
    let elapsed = start.elapsed();
    drop(synced);
    drop(connection);
    Ok(elapsed)
}

fn bench_memory(c: &mut Criterion) -> TestResult {
    let project = tempfile::tempdir().check("create project dir")?;
    common::generate_fixture(project.path(), common::LARGE)?.log("memory");
    let failure = FirstFailure::new();

    // Open the just-built database, do one full publish, sample peak RSS
    // *during* the publish (criterion's bench_function sleeps between
    // samples, so we re-record inside `iter_custom` to capture the peak
    // of every iteration rather than the steady-state sample).
    c.bench_function("peak_memory_after_full_sync_200", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;
            for _ in 0..iters {
                match timed_full_sync(project.path()) {
                    Ok(elapsed) => total += elapsed,
                    Err(error) => {
                        // Stop this sample; the error is reported once at
                        // the bench entry point.
                        failure.record::<()>(Err(error));
                        break;
                    }
                }
            }
            total
        });
    });

    // Final measurement of peak RSS after a single sync — criterion's
    // bench loop already ran many, so this reports the steady-state
    // peak as a side-channel.
    failure.finish()?;
    match peak_rss_kib().check("measure peak RSS for peak_memory_after_full_sync_200")? {
        Some(peak_kib) => {
            eprintln!("peak_memory_after_full_sync_200: {peak_kib} KiB peak RSS");
        }
        None => eprintln!(
            "peak_memory_after_full_sync_200: VmHWM unavailable on this OS (not linux); \
             bench ran but the memory figure is not recorded."
        ),
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut criterion = Criterion::default().configure_from_args();
    let result = bench_memory(&mut criterion);
    criterion.final_summary();
    exit_code(result)
}
