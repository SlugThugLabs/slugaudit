#![forbid(unsafe_code)]

//! Large-repo soak: a synthetic 10k-file project pushed through the real
//! sync path (`SourceSyncManager::ensure_current`), asserting that the
//! import completes *inside* `ResourceLimits::max_sync_wall_clock` and that
//! the index it leaves behind actually covers the file set.
//!
//! Nothing else in this repository exercises an import at this size. The
//! budget that aborts a too-slow import (`SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`,
//! default 1200 s for the default `adaptive` profile) is otherwise only
//! exercised by `src/sync/timeout_tests.rs`, which sets the budget to one
//! nanosecond to prove the *error path* fires. So the repository can prove a
//! spent budget is reported, and cannot prove a legitimate large import fits.
//! This closes that gap: it fails if a change makes a 10k-file publish exceed
//! the budget (the sync would return `sync_budget_exhausted`), and it fails if
//! a change makes the publish complete while silently dropping files from the
//! index.
//!
//! # Two sizes, one pipeline
//!
//! The assertions live in one shared runner, `sync_synthetic_project`, and
//! two tests call it:
//!
//! - `a_sixty_four_file_project_syncs_inside_the_default_wall_clock_budget`
//!   runs in the default suite. It is the same pipeline at a size every push
//!   can afford (a fraction of a second), so a regression in the assertions
//!   themselves is caught immediately, not only by whoever remembers to run
//!   the soak. It also keeps the shared runner covered: an `#[ignore]`d body
//!   is still instrumented, so a soak-only file would land in
//!   `check_coverage`'s `--all-targets` report as a hundred-odd uncovered
//!   lines, pressing on a gate whose threshold was deliberately set to the
//!   measured coverage (the 2026-08-12 recalibration in
//!   `.planning/archive/DECISIONS.md`).
//! - `a_ten_thousand_file_project_syncs_inside_the_default_wall_clock_budget`
//!   is the soak itself and is `#[ignore]`d.
//!
//! # Why `tests/` with `#[ignore]`, not a bench
//!
//! The assertion is a correctness claim (completed + complete), not a
//! measurement, so it belongs in the test harness where a failure is a red
//! test rather than a criterion outlier a reader has to interpret.
//! `#[ignore]` is what keeps the soak off the default suite: CI's
//! `cargo test --lib --bins --tests` and a plain `cargo test` compile this
//! crate and skip the soak body, so the only per-push price is the small
//! companion test above. A `benches/` target would instead have to be
//! registered in `Cargo.toml`'s `[[bench]]` list, and as a criterion target
//! it would join the `--all-targets` set the coverage gate drives on every
//! push — so the 10k sync would either execute inside the coverage step or
//! land in its report as a large block of uncovered lines, neither of which
//! belongs in a gate whose job is to compare a commit against a baseline.
//! It would also sit in the same namespace `check_performance` compares
//! against `perf_baseline.json`.
//!
//! # Running it
//!
//! ```text
//! cargo test --test large_repo_soak -- --ignored --nocapture
//! ```
//!
//! Not scheduled in CI. Measured on the machine it was written on (12-core
//! Ryzen 9 5900X, Linux, unoptimized `test` profile): two runs of the soak
//! synced all 10k files in 21.3 s and 23.6 s, against the 1200 s default
//! budget. That is 2 percent of the budget here, but the 10k-file
//! write/hash/parse/publish work is exactly the kind of thing a shared
//! GitHub runner would turn into a slow, load-sensitive, occasionally flaky
//! job — and a soak test that is skipped or flaky is worth less than one a
//! maintainer runs deliberately. Run it on demand when the discovery,
//! hashing, parsing, or publish path changes — the same trigger
//! `check_performance` documents.

use std::fs;
use std::path::Path;
use std::time::Instant;

use slugaudit_mcp_rust::model::{AuditProfile, limits_for_profile, process_limits};
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::store::open_read_only;
use slugaudit_mcp_rust::sync::SourceSyncManager;
use tempfile::TempDir;

/// Files the soak project contains. The plan's target is "~10k".
const SOAK_FILES: usize = 10_000;

/// Files the default-suite companion project contains: enough to walk,
/// hash, parse, publish, and verify more than one module directory, without
/// a soak-sized run.
const COMPANION_FILES: usize = 64;

/// A tempdir with the `.planning/slugaudit` activation marker — the minimum
/// shape of an enabled project (`project::resolve_project`).
fn activated_project() -> TempDir {
    let project = tempfile::tempdir().expect("create project temp dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("create activation dir");
    project
}

/// Writes `files` small but genuinely parseable Rust modules spread over
/// `directories` directories, and returns their project-relative, sorted
/// paths.
fn write_synthetic_project(root: &Path, files: usize, directories: usize) -> Vec<String> {
    assert!(
        directories > 0 && files.is_multiple_of(directories),
        "the synthetic project must divide evenly: {files} files over {directories} directories"
    );
    let mut expected = Vec::with_capacity(files);
    for directory in 0..directories {
        let relative_directory = format!("src/mod_{directory:03}");
        fs::create_dir_all(root.join(&relative_directory))
            .expect("create synthetic module directory");
        for file in 0..files / directories {
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
                 \x20       seed.wrapping_mul({})\n\
                 \x20           .wrapping_add(self.labels.len() as u64)\n\
                 \x20   }}\n\
                 }}\n\
                 \n\
                 pub fn entry_{directory}_{file}(seed: u64) -> Record{directory}_{file} {{\n\
                 \x20   Record{directory}_{file} {{ id: seed, labels: HashMap::new() }}\n\
                 }}\n",
                directory + 1
            );
            fs::write(root.join(&relative), source).expect("write synthetic source file");
            expected.push(relative);
        }
    }
    expected.sort();
    expected
}

/// Count of rows matching `sql`.
fn count(connection: &rusqlite::Connection, sql: &str) -> usize {
    let value: i64 = connection
        .query_row(sql, [], |row| row.get(0))
        .expect("count rows");
    usize::try_from(value).expect("row count fits in usize")
}

/// Every stored `files.path`, sorted.
fn stored_paths(connection: &rusqlite::Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .expect("prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<Vec<String>, _>>()
        .expect("collect stored paths")
}

/// Builds the fixture, syncs it through `SourceSyncManager::ensure_current`,
/// and asserts the three things the plan asks for: the sync completed, it
/// stayed inside `max_sync_wall_clock`, and the index covers the whole file
/// set (row count, file kinds, parse outcomes, and the exact path set).
fn sync_synthetic_project(files: usize, directories: usize) {
    let project = activated_project();
    let expected = write_synthetic_project(project.path(), files, directories);
    assert_eq!(expected.len(), files, "fixture wrote the wrong file count");

    // The budget the sync itself will enforce: whatever `process_limits()`
    // resolved (env overrides included). The profile default is printed
    // alongside it so a maintainer can see whether an override was in play.
    let budget = process_limits().max_sync_wall_clock;
    let profile_default = limits_for_profile(AuditProfile::default()).max_sync_wall_clock;

    let manager = SourceSyncManager::new();
    let started = Instant::now();
    let synced = manager.ensure_current(&project.path().to_string_lossy(), &NoopProgressSink);
    let elapsed = started.elapsed();

    let synced = match synced {
        Ok(synced) => synced,
        Err(error) => panic!(
            "sync of {files} files did not complete after {elapsed:?} \
             (budget {budget:?}, profile default {profile_default:?}): {error}"
        ),
    };
    assert!(
        elapsed < budget,
        "sync of {files} files completed in {elapsed:?}, outside the {budget:?} budget"
    );

    let connection = open_read_only(&synced.database_path).expect("open the published database");
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM files"),
        files,
        "the index does not cover the fixture: {files} files were written but the \
         files table holds a different number of rows"
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE file_kind = 'indexed'"
        ),
        files,
        "some fixture files were discovered but not indexed (binary/skipped rows)"
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM files WHERE parse_outcome = 'Succeeded'"
        ),
        files,
        "some fixture files were indexed without a successful parse"
    );
    assert_eq!(
        stored_paths(&connection),
        expected,
        "the stored path set differs from the fixture's file set"
    );

    eprintln!(
        "synthetic sync: {files} files published and verified in {:.3}s \
         (enforced budget {budget:?}; {} profile default {profile_default:?}; revision {})",
        elapsed.as_secs_f64(),
        match AuditProfile::default() {
            AuditProfile::Adaptive => "adaptive",
            AuditProfile::Lean => "lean",
            AuditProfile::Audit => "audit",
        },
        synced.revision_id,
    );
}

/// The default-suite companion: the same pipeline and the same assertions at
/// a size every push can afford.
#[test]
fn a_sixty_four_file_project_syncs_inside_the_default_wall_clock_budget() {
    sync_synthetic_project(COMPANION_FILES, 8);
}

/// The soak. Deliberately `#[ignore]`d so the 10k-file import is a decision a
/// maintainer makes, not a tax on every push.
#[test]
#[ignore = "10k-file soak; run on demand with `cargo test --test large_repo_soak -- --ignored --nocapture`"]
fn a_ten_thousand_file_project_syncs_inside_the_default_wall_clock_budget() {
    sync_synthetic_project(SOAK_FILES, 100);
}
