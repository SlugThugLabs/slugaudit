//! Stopping conditions inside `mechanic::scan`: wall-clock deadlines between
//! files and `QueryCursor` match-limit overflow within a file.

use super::scan_tests::{OVERFLOW_LET_QUERY, dense_let_fn};
use super::tests::{EVAL_QUERY, add, call, path_of, python_project};
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::page::PageLimit;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn a_scan_stopped_between_files_reports_the_last_file_it_finished() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let root = crate::tools::project_root(&path).check("project root")?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let catalog = mechanic_catalog::load(manager.session_mechanics(), root.as_path())
        .check("load the mechanic catalog")?;
    let synced =
        crate::tools::context::ensure_synced(&path, &NoopProgressSink, &manager).check("sync")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
    let mut delivered = Vec::new();

    let end = crate::tools::context::with_verified_read(&synced, |tx| {
        super::scan::scan(
            tx,
            &catalog.custom,
            &[0],
            None,
            Some(deadline),
            &mut |hit| {
                delivered.push((hit.file.to_owned(), hit.line));
                std::thread::sleep(std::time::Duration::from_millis(400));
                Ok(())
            },
        )
    })
    .check("a stopped scan is not an error")?;

    ensure_eq(
        &end,
        &super::scan::ScanEnd::Stopped(super::scan::Stop {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: Some("app.py".to_owned()),
            files_scanned: 1,
            files_total: 2,
        }),
        "scan stopped between files reports TimeBudget and app.py",
    )?;
    ensure(
        delivered.iter().all(|(file, _)| file == "app.py"),
        "only app.py hits were delivered",
    )?;
    ensure_eq(&delivered.len(), &3, "delivered.len() == 3")?;
    Ok(())
}

#[test]
fn a_scan_stopped_by_query_cursor_match_limit_reports_query_match_limit() -> TestResult {
    let project =
        crate::tools::test_support::activated_project("a_ok.rs", dense_let_fn(2).as_bytes())?;
    let path = path_of(&project)?;
    let root = crate::tools::project_root(&path).check("project root")?;
    let manager = sync::SourceSyncManager::default();
    call(
        &manager,
        add(&path, "let_quads", "rust", OVERFLOW_LET_QUERY),
    )
    .check("register on small file")?;
    std::fs::write(project.path().join("b_overflow.rs"), dense_let_fn(370))
        .check("write overflowing file")?;
    let catalog = mechanic_catalog::load(manager.session_mechanics(), root.as_path())
        .check("load catalog")?;
    let synced =
        crate::tools::context::ensure_synced(&path, &NoopProgressSink, &manager).check("sync")?;
    let mut delivered = Vec::new();

    let end = crate::tools::context::with_verified_read(&synced, |tx| {
        super::scan::scan(tx, &catalog.custom, &[0], None, None, &mut |hit| {
            delivered.push((hit.file.to_owned(), hit.line));
            Ok(())
        })
    })
    .check("match-limit stop returns ScanEnd::Stopped")?;

    ensure_eq(
        &end,
        &super::scan::ScanEnd::Stopped(super::scan::Stop {
            limited_by: PageLimit::QueryMatchLimit,
            resume_after_file: Some("a_ok.rs".to_owned()),
            files_scanned: 1,
            files_total: 2,
        }),
        "scan reports QueryMatchLimit and last completed file",
    )?;
    ensure_eq(&delivered.len(), &2, "only a_ok.rs's 2 hits are delivered")?;
    Ok(())
}
