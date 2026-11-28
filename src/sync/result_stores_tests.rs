use super::*;
use crate::store::{Cell, ResultSpec, Take};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::path::PathBuf;

fn sample_spec() -> ResultSpec {
    ResultSpec {
        tool: "query".to_owned(),
        project: "proj".to_owned(),
        revision_id: "rev-1".to_owned(),
        columns: vec!["n".to_owned()],
    }
}

#[test]
fn opening_project_b_replaces_project_a_store_in_single_slot() -> TestResult {
    let slot = ResultStoreSlot::default();
    let dir_a = tempfile::tempdir().check("tempdir A")?;
    let dir_b = tempfile::tempdir().check("tempdir B")?;
    let limits = ResultStoreLimits {
        max_bytes: 16 * 1024 * 1024,
        active_grace_secs: 900,
    };

    let store_a = slot
        .for_dir_with_limits(dir_a.path(), limits)
        .check("open store A")?;
    let weak_a = Arc::downgrade(&store_a);
    let mut writer_a = store_a.begin(sample_spec()).check("begin A")?;
    writer_a.push(&[Cell::Int(42)]).check("push A")?;
    let handle_a = writer_a
        .finish(&serde_json::json!({"kind": "complete"}))
        .check("finish A")?;
    let token_a = store_a.cursor(handle_a.id, 0).to_string();
    drop(store_a);

    let store_b = slot
        .for_dir_with_limits(dir_b.path(), limits)
        .check("open store B")?;
    let mut writer_b = store_b.begin(sample_spec()).check("begin B")?;
    writer_b.push(&[Cell::Int(99)]).check("push B")?;
    let handle_b = writer_b
        .finish(&serde_json::json!({"kind": "complete"}))
        .check("finish B")?;
    let token_b = store_b.cursor(handle_b.id, 0).to_string();

    ensure(
        weak_a.upgrade().is_none(),
        "opening project B drops project A's ResultStore and closes its descriptors",
    )?;
    ensure(
        slot.get_dir(dir_a.path()).check("get A")?.is_none(),
        "project A is no longer held in the slot",
    )?;
    ensure(
        slot.get_dir(dir_b.path()).check("get B")?.is_some(),
        "project B is now the active store",
    )?;

    let resolved_for_a = slot
        .store_for_continuation()
        .check("resolve active store")?
        .check("active store B returned")?;
    let _err = resolved_for_a
        .decode_cursor(&token_a)
        .check_err("project A cursor nonce is rejected by project B store")?;
    ensure_eq(
        &resolved_for_a.stats().check("stats B")?.unknown_cursors,
        &1,
        "foreign project cursor increments unknown_cursors on active store",
    )?;

    let decoded_b = store_b.decode_cursor(&token_b).check("decode token B")?;
    let page_b = store_b
        .read_page(decoded_b.result, decoded_b.ord, |_| Take::Yes)
        .check("read page B")?;
    ensure_eq(
        &page_b.rows.first().check("row B")?.cells,
        &vec![Cell::Int(99)],
        "project B cursor reads project B's stored row",
    )?;

    slot.release_dir(dir_a.path())
        .check("releasing non-active dir A is a no-op")?;
    ensure(
        slot.get_dir(dir_b.path()).check("get B")?.is_some(),
        "releasing dir A leaves active dir B intact",
    )?;
    slot.release_dir(dir_b.path()).check("release B")?;
    ensure(
        slot.current().check("current")?.is_none(),
        "releasing active dir B empties the slot",
    )?;
    Ok(())
}

#[test]
fn external_results_dir_deletion_and_recreation_reopens_clean_store() -> TestResult {
    let slot = ResultStoreSlot::default();
    let dir = tempfile::tempdir().check("tempdir")?;
    let limits = ResultStoreLimits {
        max_bytes: 16 * 1024 * 1024,
        active_grace_secs: 900,
    };

    let first = slot
        .for_dir_with_limits(dir.path(), limits)
        .check("initial open")?;
    let weak_first = Arc::downgrade(&first);
    drop(first);

    std::fs::remove_dir_all(dir.path().join("results")).check("external rm -rf results")?;
    ensure(
        slot.get_dir(dir.path())
            .check("get_dir after external rm")?
            .is_none(),
        "get_dir returns None when results/ directory was externally removed",
    )?;
    ensure(
        slot.current().check("current after external rm")?.is_none(),
        "current returns None when results/ directory was externally removed",
    )?;

    let reopened = slot
        .for_dir_with_limits(dir.path(), limits)
        .check("reopen after external recreation")?;
    ensure(
        weak_first.upgrade().is_none(),
        "stale store pointing at unlinked directory was dropped before reopening",
    )?;
    ensure(
        dir.path().join("results").is_dir(),
        "reopening recreated results/ directory",
    )?;
    ensure(
        slot.get_dir(dir.path())
            .check("get_dir after reopen")?
            .is_some(),
        "reopened store is live",
    )?;
    drop(reopened);
    Ok(())
}

#[test]
fn re_fetching_store_updates_its_limits_in_place() -> TestResult {
    let slot = ResultStoreSlot::default();
    let dir = tempfile::tempdir().check("tempdir")?;

    let first = slot
        .for_dir_with_limits(
            dir.path(),
            ResultStoreLimits {
                max_bytes: 1_000_000,
                active_grace_secs: 900,
            },
        )
        .check("initial open")?;
    ensure_eq(
        &first.stats().check("initial stats")?.budget_bytes,
        &1_000_000,
        "initial store budget matches first limits",
    )?;

    let second = slot
        .for_dir_with_limits(
            dir.path(),
            ResultStoreLimits {
                max_bytes: 8_000_000,
                active_grace_secs: 300,
            },
        )
        .check("second lookup")?;
    ensure(
        Arc::ptr_eq(&first, &second),
        "re-fetching the same runtime directory reuses the same ResultStore",
    )?;
    ensure_eq(
        &second.stats().check("updated stats")?.budget_bytes,
        &8_000_000,
        "re-fetching updates the live ResultStore byte budget",
    )?;
    Ok(())
}

fn lock_files_in(results_dir: &Path) -> TestResult<Vec<PathBuf>> {
    let mut locks = Vec::new();
    for entry in std::fs::read_dir(results_dir).check("read results directory")? {
        let entry = entry.check("results directory entry")?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "lock") {
            locks.push(path);
        }
    }
    locks.sort();
    Ok(locks)
}

#[test]
fn holding_old_store_across_results_removal_and_reopen_preserves_new_lock() -> TestResult {
    let slot = ResultStoreSlot::default();
    let dir = tempfile::tempdir().check("tempdir")?;
    let limits = ResultStoreLimits {
        max_bytes: 16 * 1024 * 1024,
        active_grace_secs: 900,
    };

    let old_store = slot
        .for_dir_with_limits(dir.path(), limits)
        .check("initial open")?;
    let results_dir = dir.path().join("results");
    let old_locks = lock_files_in(&results_dir)?;
    ensure_eq(
        &old_locks.len(),
        &1,
        "initial open creates one owner lock file",
    )?;
    let old_lock = old_locks.first().check("initial lock path")?.clone();

    std::fs::remove_dir_all(&results_dir).check("external removal of results/")?;

    let new_store = slot
        .for_dir_with_limits(dir.path(), limits)
        .check("reopen succeeds while old Arc<ResultStore> is still held")?;
    let new_locks = lock_files_in(&results_dir)?;
    ensure_eq(
        &new_locks.len(),
        &1,
        "reopened store creates one owner lock file",
    )?;
    let new_lock = new_locks.first().check("reopened lock path")?.clone();
    ensure_ne(
        &new_lock,
        &old_lock,
        "reopened store uses a distinct UUID lock path",
    )?;
    ensure(
        new_lock.is_file(),
        "reopened store lock file exists before dropping old store",
    )?;

    drop(old_store);

    ensure(
        new_lock.is_file(),
        "dropping old Arc<ResultStore> does not unlink reopened store's lock file",
    )?;
    let mut writer = new_store.begin(sample_spec()).check("begin on new store")?;
    writer.push(&[Cell::Int(7)]).check("push on new store")?;
    let handle = writer
        .finish(&serde_json::json!({"kind": "complete"}))
        .check("finish on new store")?;
    let token = new_store.cursor(handle.id, 0).to_string();
    let decoded = new_store.decode_cursor(&token).check("decode new cursor")?;
    let page = new_store
        .read_page(decoded.result, decoded.ord, |_| Take::Yes)
        .check("read new page")?;
    ensure_eq(
        &page.rows.first().check("row")?.cells,
        &vec![Cell::Int(7)],
        "reopened store remains operational after dropping old store",
    )?;
    let live = slot
        .get_dir(dir.path())
        .check("get active store")?
        .check("slot holds reopened store")?;
    ensure(
        Arc::ptr_eq(&live, &new_store),
        "slot continues holding the reopened store",
    )?;
    Ok(())
}
