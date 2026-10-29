//! A registered scan the call deadline stops: the engine reports how far it
//! got, and rows that cannot be reported fail the call instead of vanishing.
use super::super::tests::{EVAL_QUERY, add, call, path_of, python_project};
use super::*;
use crate::progress::NoopProgressSink;

#[test]
fn a_registered_entrys_rescan_stopped_by_the_budget_fails_naming_the_entry() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let (limits, catalog) = spent_budget(&manager, &path);
    let scope = crate::tools::leads_page::Scope {
        catalog: &catalog,
        selected: None,
        mode: LeadMode::Full,
        pattern: None,
        deadline: crate::tools::query_exec::deadline_after(limits.max_structure_execution_time),
        cache: manager.mechanic_scan_cache(),
    };
    let synced =
        crate::tools::context::ensure_synced(&path, &NoopProgressSink, &manager).expect("sync");

    let error = crate::tools::context::with_verified_read(&synced, |tx| {
        crate::tools::leads_custom::rescan(tx, &scope, 0, &mut |_, _, _| Ok(()))
    })
    .expect_err("rows that cannot be reported must not read as an empty entry");

    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "a spent budget is operational, not a contract error: {error:?}"
    );
    assert!(
        error.message.contains("`py_eval`")
            && error.message.contains("max_structure_execution_time"),
        "{}",
        error.message
    );
    let data = error.data.expect("the budget carries a code");
    assert_eq!(data["code"], "mechanic_budget_exhausted");
    assert!(
        data["remedy"]
            .as_str()
            .is_some_and(|remedy| remedy.contains("mechanic action remove")),
        "{data}"
    );
}

/// The scan engine's stop: the deadline passes while the first file's hits
/// are being delivered, so the second file is never started. The report names
/// the first file as the last one done and counts both files.
#[test]
fn a_scan_stopped_between_files_reports_the_last_file_it_finished() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let limits = crate::tools::resolve_limits(&path).expect("limits");
    let root = crate::tools::project_root(&path).expect("root");
    let catalog = mechanic_catalog::load(&root, manager.session_mechanics(), &limits);
    let synced =
        crate::tools::context::ensure_synced(&path, &NoopProgressSink, &manager).expect("sync");
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
    let mut delivered = Vec::new();

    let end = crate::tools::context::with_verified_read(&synced, |tx| {
        crate::tools::mechanic_scan::scan(
            tx,
            &catalog.custom,
            &[0],
            None,
            Some(deadline),
            &mut |hit| {
                delivered.push((hit.file.to_owned(), hit.line));
                // Spend the budget while the first file's hits are delivered.
                std::thread::sleep(std::time::Duration::from_millis(400));
                Ok(())
            },
        )
    })
    .expect("a stopped scan is not an error");

    assert_eq!(
        end,
        crate::tools::mechanic_scan::ScanEnd::Stopped(crate::tools::mechanic_scan::Stop {
            resume_after_file: Some("app.py".to_owned()),
            files_scanned: 1,
            files_total: 2,
        })
    );
    assert!(
        delivered.iter().all(|(file, _)| file == "app.py"),
        "no hit of the unstarted file: {delivered:?}"
    );
    assert_eq!(
        delivered.len(),
        3,
        "app.py's three hits, whole: {delivered:?}"
    );
}
