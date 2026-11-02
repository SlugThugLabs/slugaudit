//! Proofs that the result store's files (`<runtime>/results/*.db` and the
//! owner lock) are invisible to discovery and to the full sync that indexes
//! a project. Each test plants real store files (a real `ResultStore` opened
//! through the manager's slot, plus a stray `.db`) and drives the real code.

use super::ResultStoreSlot;
use crate::store::open_existing;
use crate::sync::test_support::{create_project, stored_paths, sync_project, write};
use slugaudit_test_check::{Check, TestResult};
use std::path::Path;

const RESULTS: &str = ".planning/slugaudit/results";

/// Opens a real store under `<root>/.planning/slugaudit` (which creates the
/// results directory and the owner lock) and plants a stray result file.
/// Returns the slot so the store (and its lock) stays alive, and the
/// project-relative paths of everything now under the results directory.
fn plant_store_files(root: &Path) -> TestResult<(ResultStoreSlot, Vec<String>)> {
    let slot = ResultStoreSlot::default();
    let runtime_dir = root.join(".planning/slugaudit");
    std::fs::create_dir_all(&runtime_dir).check("create the runtime directory")?;
    slot.for_dir(&runtime_dir)
        .check("open the store under the project")?;
    write(root, &format!("{RESULTS}/stray-result.db"), b"not a result")?;
    let mut planted: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root.join(RESULTS)).check("list the results directory")? {
        planted.push(format!(
            "{RESULTS}/{}",
            entry.check("results entry")?.file_name().to_string_lossy()
        ));
    }
    planted.sort();
    assert!(
        planted.iter().any(|path| has_extension(path, "lock")),
        "the owner lock exists: {planted:?}"
    );
    assert!(
        planted.iter().any(|path| path.ends_with("stray-result.db")),
        "the stray result file exists: {planted:?}"
    );
    Ok((slot, planted))
}

/// Whether `path` names a file whose extension is `extension`, ignoring ASCII case.
fn has_extension(path: &str, extension: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

fn mentions_results(path: &str) -> bool {
    path.contains("results") || path.contains(".lock") || has_extension(path, "db")
}

#[test]
fn result_store_proof_results_dir_and_lock_file_are_not_discovered() -> TestResult {
    let project = create_project()?;
    write(project.path(), "lib.rs", b"pub fn a() {}\n")?;
    let (_slot, planted) = plant_store_files(project.path())?;
    let manager = crate::sync::SourceSyncManager::new();

    let synced = sync_project(&manager, &project)?;

    let connection = open_existing(&synced.database_path).check("open the index")?;
    let paths = stored_paths(&connection)?;
    assert_eq!(paths, ["lib.rs"], "planted files: {planted:?}");
    let (discovered, _skipped) =
        crate::sync::discovery::discover(project.path()).check("discovery succeeds")?;
    let discovered: Vec<String> = discovered
        .into_iter()
        .map(|file| file.relative_path)
        .collect();
    assert!(
        discovered.iter().all(|path| !mentions_results(path)),
        "discovered: {discovered:?}"
    );
    Ok(())
}

/// The sync after the store opens must not index the store: the results
/// directory and the owner lock stay outside the file set while a real edit
/// still publishes.
#[test]
fn result_store_proof_sync_after_the_store_opens_does_not_index_it() -> TestResult {
    let manager = crate::sync::SourceSyncManager::new();
    let project = create_project()?;
    write(project.path(), "lib.rs", b"pub fn a() {}\n")?;
    let first = sync_project(&manager, &project)?;
    let (_slot, planted) = plant_store_files(project.path())?;
    write(project.path(), "lib.rs", b"pub fn a_longer_name() {}\n")?;

    let second = manager
        .ensure_current(
            &project.path().to_string_lossy(),
            &crate::progress::NoopProgressSink,
        )
        .check("sync succeeds")?;

    assert_ne!(first.revision_id, second.revision_id, "the edit published");
    let connection = open_existing(&second.database_path).check("open the index")?;
    assert_eq!(
        stored_paths(&connection)?,
        ["lib.rs"],
        "planted files: {planted:?}"
    );
    Ok(())
}
