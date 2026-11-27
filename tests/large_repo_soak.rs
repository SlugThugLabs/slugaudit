#![forbid(unsafe_code)]

//! Large-repo soak: a synthetic 10k-file project pushed through the real
//! sync path (`SourceSyncManager::ensure_current` and `sync::publish`),
//! asserting that the cold import, the warm unchanged-tree pass, and a
//! single-file incremental edit each complete *inside*
//! `ResourceLimits::max_sync_wall_clock` with exact delta counts and index
//! coverage.
//!
//! # Two sizes, one pipeline
//!
//! The assertions live in one shared runner, `sync_synthetic_project`, and
//! two tests call it:
//!
//! - `a_sixty_four_file_project_syncs_inside_the_default_wall_clock_budget`
//!   runs in the default CI suite at 64 files across 8 directories (cold
//!   import, warm unchanged-tree verification, and single-file incremental
//!   edit), catching regressions in sync correctness or assertions on every
//!   push.
//! - `a_ten_thousand_file_project_syncs_inside_the_default_wall_clock_budget`
//!   is the 10k-file soak itself and is `#[ignore]`d for on-demand runs:
//!
//! ```text
//! cargo test --test large_repo_soak -- --ignored --nocapture
//! ```

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use slugaudit_mcp_rust::model::{AuditProfile, limits_for_profile};
use slugaudit_mcp_rust::parse::PACK_VERSION;
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::project::effective_limits;
use slugaudit_mcp_rust::store::{open_existing, open_read_write};
use slugaudit_mcp_rust::sync::{SourceSyncManager, SyncedProject, publish};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq, ensure_ne};
use tempfile::TempDir;

/// Files the soak project contains (~10k).
const SOAK_FILES: usize = 10_000;

/// Files the default-suite companion project contains: enough to walk,
/// hash, parse, publish, and verify multiple module directories quickly.
const COMPANION_FILES: usize = 64;

/// Peak RSS ceiling (`VmHWM` in `/proc/self/status`) across the soak process:
/// 512 MiB (`524_288` KiB).
#[cfg(target_os = "linux")]
const MAX_PEAK_RSS_KIB: u64 = 512 * 1024;

/// Shared parameters across the cold, warm, and incremental soak phases.
struct SoakFixture<'a> {
    root: &'a Path,
    project_path: &'a str,
    files: usize,
    budget: Duration,
    warm_budget: Duration,
    incremental_budget: Duration,
    profile_default: Duration,
}

/// A tempdir with the `.planning/slugaudit` activation directory — the sole
/// activation marker required by `project::resolve_project`.
fn activated_project() -> TestResult<TempDir> {
    let project = tempfile::tempdir().check("create project temp dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("create activation dir")?;
    Ok(project)
}

/// Writes `files` small but genuinely parseable Rust modules spread over
/// `directories` directories, and returns their project-relative, sorted
/// paths.
fn write_synthetic_project(
    root: &Path,
    files: usize,
    directories: usize,
) -> TestResult<Vec<String>> {
    ensure(
        directories > 0 && files.is_multiple_of(directories),
        format!(
            "the synthetic project must divide evenly: {files} files over {directories} directories"
        ),
    )?;
    let mut expected = Vec::with_capacity(files);
    let files_per_directory = files
        .checked_div(directories)
        .check("the directory count was checked non-zero above")?;
    for directory in 0..directories {
        let multiplier = directory
            .checked_add(1)
            .check("the module multiplier overflowed")?;
        let relative_directory = format!("src/mod_{directory:03}");
        fs::create_dir_all(root.join(&relative_directory))
            .check("create synthetic module directory")?;
        for file in 0..files_per_directory {
            let relative = format!("{relative_directory}/file_{file:03}.rs");
            let source = format!(
                "//! Synthetic module {directory}/{file}.\n\
                 use std::collections::HashMap;\n\
                 \n\
                 pub struct Record{directory}_{file} {{\n\
                 \x20   pub id: u64,\n\
                 \x20   pub labels: HashMap<String, u64>,\n\
                 }}\n\
                 \n\
                 impl Record{directory}_{file} {{\n\
                 \x20   pub fn score(&self, seed: u64) -> u64 {{\n\
                 \x20       seed.wrapping_mul({multiplier})\n\
                 \x20           .wrapping_add(self.labels.len() as u64)\n\
                 \x20   }}\n\
                 }}\n\
                 \n\
                 pub fn entry_{directory}_{file}(seed: u64) -> Record{directory}_{file} {{\n\
                 \x20   Record{directory}_{file} {{ id: seed, labels: HashMap::new() }}\n\
                 }}\n"
            );
            fs::write(root.join(&relative), source).check("write synthetic source file")?;
            expected.push(relative);
        }
    }
    expected.sort();
    Ok(expected)
}

/// Count of rows matching `sql`.
fn count(connection: &rusqlite::Connection, sql: &str) -> TestResult<usize> {
    let value: i64 = connection
        .query_row(sql, [], |row| row.get(0))
        .check("count rows")?;
    usize::try_from(value).check("row count fits in usize")
}

/// Every stored `files.path`, sorted.
fn stored_paths(connection: &rusqlite::Connection) -> TestResult<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .check("prepare stored paths query")?;
    statement
        .query_map([], |row| row.get(0))
        .check("query stored paths")?
        .collect::<Result<Vec<String>, _>>()
        .check("collect stored paths")
}

/// Phase 1: cold initial sync through `SourceSyncManager::ensure_current` and
/// full index verification.
fn verify_cold_sync(
    fixture: &SoakFixture<'_>,
    manager: &SourceSyncManager,
    expected: &[String],
) -> TestResult<(SyncedProject, Duration)> {
    let cold_started = Instant::now();
    let synced = manager.ensure_current(fixture.project_path, &NoopProgressSink);
    let cold_elapsed = cold_started.elapsed();

    let synced = match synced {
        Ok(synced) => synced,
        Err(error) => {
            return Err(format!(
                "cold sync of {} files did not complete after {cold_elapsed:?} \
                 (budget {:?}, profile default {:?}): {error}",
                fixture.files, fixture.budget, fixture.profile_default
            ))
            .check("cold sync must succeed");
        }
    };
    ensure(
        cold_elapsed < fixture.budget,
        format!(
            "cold sync of {} files completed in {cold_elapsed:?}, outside the {:?} budget",
            fixture.files, fixture.budget
        ),
    )?;

    let connection = open_existing(&synced.database_path).check("open the published database")?;
    ensure_eq(
        &count(&connection, "SELECT COUNT(*) FROM files")?,
        &fixture.files,
        "files table row count matches the fixture file count",
    )?;
    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE file_kind = 'indexed'",
        )?,
        &fixture.files,
        "every synthetic Rust file must be indexed with content",
    )?;
    ensure_eq(
        &count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE parse_outcome = 'Succeeded'",
        )?,
        &fixture.files,
        "every synthetic Rust file must parse cleanly",
    )?;
    ensure_eq(
        &stored_paths(&connection)?.as_slice(),
        &expected,
        "the stored path set matches the fixture's file set",
    )?;
    Ok((synced, cold_elapsed))
}

/// Phase 2: warm unchanged-tree sync (`added == 0, modified == 0, deleted == 0`).
fn verify_warm_unchanged_sync(
    fixture: &SoakFixture<'_>,
    manager: &SourceSyncManager,
    synced: &SyncedProject,
) -> TestResult<Duration> {
    let mut rw_connection =
        open_read_write(&synced.database_path).check("open database for warm unchanged sync")?;
    let warm_started = Instant::now();
    let warm_report = publish(
        &mut rw_connection,
        fixture.root,
        PACK_VERSION,
        &NoopProgressSink,
    )
    .check("warm unchanged publish must succeed")?;
    let warm_elapsed = warm_started.elapsed();
    drop(rw_connection);

    ensure(
        warm_elapsed < fixture.warm_budget,
        format!(
            "warm unchanged sync of {} files took {warm_elapsed:?}, outside the {:?} warm budget (sync budget {:?})",
            fixture.files, fixture.warm_budget, fixture.budget
        ),
    )?;
    ensure_eq(
        &warm_report.added,
        &0,
        "warm unchanged sync must add zero files",
    )?;
    ensure_eq(
        &warm_report.modified,
        &0,
        "warm unchanged sync must modify zero files",
    )?;
    ensure_eq(
        &warm_report.deleted,
        &0,
        "warm unchanged sync must delete zero files",
    )?;
    ensure_eq(
        &warm_report.unchanged,
        &fixture.files,
        "warm unchanged sync must keep every file unchanged",
    )?;
    ensure(
        warm_report.skipped.is_empty(),
        "warm unchanged sync must skip zero files",
    )?;
    ensure_eq(
        &warm_report.revision_id,
        &synced.revision_id,
        "warm unchanged sync must preserve the committed revision id",
    )?;

    let manager_warm_started = Instant::now();
    let warm_synced = manager
        .ensure_current(fixture.project_path, &NoopProgressSink)
        .check("warm manager ensure_current must succeed")?;
    let manager_warm_elapsed = manager_warm_started.elapsed();
    ensure(
        manager_warm_elapsed < fixture.warm_budget,
        format!(
            "warm ensure_current of {} files took {manager_warm_elapsed:?}, outside {:?} (sync budget {:?})",
            fixture.files, fixture.warm_budget, fixture.budget
        ),
    )?;
    ensure_eq(
        &warm_synced.revision_id,
        &synced.revision_id,
        "warm ensure_current keeps the unchanged revision id",
    )?;
    Ok(warm_elapsed)
}

/// Appends one new Rust function to `edited_relative` under `root`.
fn append_incremental_probe(root: &Path, edited_relative: &str) -> TestResult {
    let mut edited_file = fs::OpenOptions::new()
        .append(true)
        .open(root.join(edited_relative))
        .check("open first fixture file for incremental edit")?;
    edited_file
        .write_all(b"\npub fn incremental_soak_probe() -> u64 {\n    99\n}\n")
        .check("append new function to edited fixture file")?;
    edited_file.flush().check("flush edited fixture file")
}

/// Verifies the SQLite `files` table holds `files` rows and the updated text of
/// `edited_relative` after an incremental publish.
fn verify_edited_file_in_store(
    database_path: &Path,
    files: usize,
    edited_relative: &str,
) -> TestResult {
    let connection = open_existing(database_path).check("reopen published database after edit")?;
    ensure_eq(
        &count(&connection, "SELECT COUNT(*) FROM files")?,
        &files,
        "file count remains exact after single-file edit",
    )?;
    let stored_edited_content: String = connection
        .query_row(
            "SELECT content FROM files WHERE path = ?1",
            [edited_relative],
            |row| row.get(0),
        )
        .check("read updated content of edited file")?;
    ensure(
        stored_edited_content.contains("incremental_soak_probe"),
        "stored content of edited file reflects the incremental edit",
    )
}

/// Phase 3: single-file edit incremental sync (`modified == 1, unchanged == files - 1`).
fn verify_single_file_edit_sync(
    fixture: &SoakFixture<'_>,
    manager: &SourceSyncManager,
    synced: &SyncedProject,
    edited_relative: &str,
) -> TestResult<(String, Duration)> {
    append_incremental_probe(fixture.root, edited_relative)?;

    let expected_unchanged = fixture
        .files
        .checked_sub(1)
        .check("synthetic fixture has at least one file")?;
    let mut rw_connection = open_read_write(&synced.database_path)
        .check("open database for single-file incremental sync")?;
    let edit_started = Instant::now();
    let edit_report = publish(
        &mut rw_connection,
        fixture.root,
        PACK_VERSION,
        &NoopProgressSink,
    )
    .check("single-file incremental publish must succeed")?;
    let edit_elapsed = edit_started.elapsed();
    drop(rw_connection);

    ensure(
        edit_elapsed < fixture.incremental_budget,
        format!(
            "single-file edit sync of {} files took {edit_elapsed:?}, outside {:?} incremental budget (sync budget {:?})",
            fixture.files, fixture.incremental_budget, fixture.budget
        ),
    )?;
    ensure_eq(
        &edit_report.added,
        &0,
        "single-file edit sync must add zero files",
    )?;
    ensure_eq(
        &edit_report.modified,
        &1,
        "single-file edit sync must modify exactly one file",
    )?;
    ensure_eq(
        &edit_report.deleted,
        &0,
        "single-file edit sync must delete zero files",
    )?;
    ensure_eq(
        &edit_report.unchanged,
        &expected_unchanged,
        "single-file edit sync must leave all unedited files unchanged",
    )?;
    ensure(
        edit_report.skipped.is_empty(),
        "single-file edit sync must skip zero files",
    )?;
    ensure_ne(
        &edit_report.revision_id,
        &synced.revision_id,
        "single-file edit sync must commit a new revision id",
    )?;

    let post_edit_synced = manager
        .ensure_current(fixture.project_path, &NoopProgressSink)
        .check("ensure_current after incremental publish must succeed")?;
    ensure_eq(
        &post_edit_synced.revision_id,
        &edit_report.revision_id,
        "ensure_current observes the incremental edit's revision without extra churn",
    )?;
    verify_edited_file_in_store(&synced.database_path, fixture.files, edited_relative)?;
    Ok((edit_report.revision_id, edit_elapsed))
}

/// Builds the fixture and exercises three consecutive real sync phases:
/// 1. Cold import through `SourceSyncManager::ensure_current`
/// 2. Warm unchanged-tree sync (`added == 0, modified == 0, deleted == 0`)
/// 3. Single-file incremental edit (`modified == 1, unchanged == files - 1`)
fn sync_synthetic_project(files: usize, directories: usize) -> TestResult {
    let project = activated_project()?;
    let expected = write_synthetic_project(project.path(), files, directories)?;
    ensure_eq(
        &expected.len(),
        &files,
        "fixture wrote the expected number of Rust source files",
    )?;

    let budget = effective_limits(project.path())
        .check("resolve effective project limits")?
        .max_sync_wall_clock;
    let profile_default = limits_for_profile(AuditProfile::default())
        .check("resolve profile limits")?
        .max_sync_wall_clock;

    let (warm_cap, incremental_cap) = if files <= COMPANION_FILES {
        (Duration::from_secs(5), Duration::from_secs(5))
    } else {
        (Duration::from_secs(15), Duration::from_secs(20))
    };
    let project_path = project.path().to_string_lossy().into_owned();
    let fixture = SoakFixture {
        root: project.path(),
        project_path: &project_path,
        files,
        budget,
        warm_budget: budget.min(warm_cap),
        incremental_budget: budget.min(incremental_cap),
        profile_default,
    };
    let manager = SourceSyncManager::new();

    let (synced, cold_elapsed) = verify_cold_sync(&fixture, &manager, &expected)?;
    let warm_elapsed = verify_warm_unchanged_sync(&fixture, &manager, &synced)?;
    let edited_relative = expected
        .first()
        .check("synthetic fixture contains at least one file")?;
    let (edit_revision_id, edit_elapsed) =
        verify_single_file_edit_sync(&fixture, &manager, &synced, edited_relative)?;
    assert_peak_rss_within_ceiling()?;

    eprintln!(
        "synthetic sync ({files} files): cold={:.3}s ({}), warm_unchanged={:.3}s [cap {:?}], \
         single_file_edit={:.3}s ({edit_revision_id}) [cap {:?}; sync budget {budget:?}; {} default {profile_default:?}]",
        cold_elapsed.as_secs_f64(),
        synced.revision_id,
        warm_elapsed.as_secs_f64(),
        fixture.warm_budget,
        edit_elapsed.as_secs_f64(),
        fixture.incremental_budget,
        match AuditProfile::default() {
            AuditProfile::Adaptive => "adaptive",
            AuditProfile::Lean => "lean",
            AuditProfile::Audit => "audit",
        },
    );
    Ok(())
}

/// Reads `/proc/self/status` on Linux and asserts `VmHWM` stays at or below
/// `512 MiB` (`524_288` KiB).
#[cfg(target_os = "linux")]
fn assert_peak_rss_within_ceiling() -> TestResult {
    let status = fs::read_to_string("/proc/self/status").check("read /proc/self/status")?;
    let hwm_line = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .check("VmHWM entry present in /proc/self/status")?;
    let kib_token = hwm_line
        .trim_start_matches("VmHWM:")
        .split_whitespace()
        .next()
        .check("VmHWM numeric KiB field")?;
    let peak_rss_kib: u64 = kib_token.parse().check("parse VmHWM KiB as u64")?;
    ensure(
        peak_rss_kib <= MAX_PEAK_RSS_KIB,
        format!(
            "peak RSS VmHWM={peak_rss_kib} KiB exceeded the {MAX_PEAK_RSS_KIB} KiB (512 MiB) ceiling"
        ),
    )
}

#[cfg(not(target_os = "linux"))]
fn assert_peak_rss_within_ceiling() -> TestResult {
    Ok(())
}

/// The default-suite companion: cold import, warm unchanged-tree check, and
/// single-file incremental edit at a size every push can afford.
#[test]
fn a_sixty_four_file_project_syncs_inside_the_default_wall_clock_budget() -> TestResult {
    sync_synthetic_project(COMPANION_FILES, 8)
}

/// The 10k-file soak. Deliberately `#[ignore]`d so the full import runs on
/// demand rather than on every push.
#[test]
#[ignore = "10k-file soak; run on demand with `cargo test --test large_repo_soak -- --ignored --nocapture`"]
fn a_ten_thousand_file_project_syncs_inside_the_default_wall_clock_budget() -> TestResult {
    sync_synthetic_project(SOAK_FILES, 100)
}
