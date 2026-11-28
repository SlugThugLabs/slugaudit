//! Stopping conditions inside `mechanic::scan`: wall-clock deadlines between
//! files and `QueryCursor` match-limit overflow within a file.

use super::scan_tests::{OVERFLOW_LET_QUERY, dense_let_fn};
use super::tests::{EVAL_QUERY, add, call, path_of, python_project};
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::context::SyncedProject;
use crate::tools::leads::MechanicCount;
use crate::tools::page::{Completeness, PageLimit};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::collections::HashSet;
use std::sync::Arc;

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
            files_scanned: 1,
            files_total: 2,
            covered_languages: HashSet::from(["python".to_owned()]),
        }),
        "scan stopped between files reports TimeBudget",
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
            files_scanned: 1,
            files_total: 2,
            covered_languages: HashSet::from(["rust".to_owned()]),
        }),
        "scan reports QueryMatchLimit",
    )?;
    ensure_eq(&delivered.len(), &2, "only a_ok.rs's 2 hits are delivered")?;
    Ok(())
}

fn partial_stop_multi_lang_fixture() -> TestResult<(tempfile::TempDir, SyncedProject)> {
    let project =
        crate::tools::test_support::activated_project("a_ok.rs", dense_let_fn(2).as_bytes())?;
    std::fs::write(project.path().join("b_overflow.rs"), dense_let_fn(370))
        .check("write overflowing file")?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    let synced =
        crate::tools::context::ensure_synced(&path, &NoopProgressSink, &manager).check("sync")?;
    let rw = crate::store::open_existing_read_write(&synced.database_path)
        .check("open project.db read-write")?;
    rw.execute(
        "INSERT INTO files (
            path, file_kind, file_class, content_hash, hash_algorithm,
            byte_len, modified_unix_seconds, language, language_detected,
            parser_availability, parse_outcome, extraction_completeness,
            last_revision_id, content
        ) VALUES (
            'app.py', 'indexed', 'authored', 'pyhash', 'blake3-v1',
            10, 0, 'python', 1,
            'Available', 'Succeeded', 'Full',
            (SELECT last_revision_id FROM files LIMIT 1), 'eval(\"1\")\n'
        )",
        [],
    )
    .check("insert parsed python row")?;
    Ok((project, synced))
}

fn partial_stop_catalog() -> TestResult<mechanic_catalog::Catalog> {
    let grammar = crate::tools::grammar::load_grammar("rust").check("load rust grammar")?;
    let query =
        Arc::new(tree_sitter::Query::new(&grammar, OVERFLOW_LET_QUERY).check("compile query")?);
    Ok(mechanic_catalog::Catalog {
        custom: vec![
            mechanic_catalog::Registered {
                entry: crate::project::MechanicEntry {
                    id: "let_quads".to_owned(),
                    language: "rust".to_owned(),
                    query: OVERFLOW_LET_QUERY.to_owned(),
                    note: "paired lets".to_owned(),
                },
                source: MechanicSource::Session,
                query: Arc::clone(&query),
            },
            mechanic_catalog::Registered {
                entry: crate::project::MechanicEntry {
                    id: "py_eval".to_owned(),
                    language: "python".to_owned(),
                    query: OVERFLOW_LET_QUERY.to_owned(),
                    note: "unreached python mechanic".to_owned(),
                },
                source: MechanicSource::Session,
                query,
            },
        ],
    })
}

fn check_partial_custom_scan(counts: &[MechanicCount], completeness: &Completeness) -> TestResult {
    ensure_eq(
        completeness,
        &Completeness::Partial {
            limited_by: PageLimit::QueryMatchLimit,
            resume_after_file: None,
            files_scanned: 1,
            files_total: 3,
        },
        "custom scan reports partial completeness after 1 of 3 files",
    )?;
    ensure_eq(&counts.len(), &2, "both selected custom mechanics counted")?;
    let rust_count = counts.first().check("rust mechanic count")?;
    ensure_eq(&rust_count.mechanic.as_str(), &"let_quads", "rust entry")?;
    ensure_eq(&rust_count.count, &2, "rust count == 2")?;
    ensure_eq(&rust_count.files, &1, "rust files == 1")?;
    ensure_eq(
        &rust_count.covers,
        &["rust"],
        "scanned rust language retains covers: [\"rust\"]",
    )?;
    let py_count = counts.get(1).check("python mechanic count")?;
    ensure_eq(&py_count.mechanic.as_str(), &"py_eval", "python entry")?;
    ensure_eq(&py_count.count, &0, "python count == 0")?;
    ensure_eq(&py_count.files, &0, "python files == 0")?;
    ensure(
        py_count.covers.is_empty(),
        format!(
            "unscanned python language must report empty covers, got {:?}",
            py_count.covers
        ),
    )?;
    Ok(())
}

#[test]
fn partial_stop_marks_only_scanned_languages_as_covered_and_unscanned_as_empty() -> TestResult {
    let (_project, synced) = partial_stop_multi_lang_fixture()?;
    let catalog = partial_stop_catalog()?;

    let (end, partial, zero_budget) = crate::tools::context::with_verified_read(&synced, |tx| {
        let end = super::scan::scan(tx, &catalog.custom, &[0, 1], None, None, &mut |_| Ok(()))?;
        let partial =
            crate::tools::leads::custom::scan_entries_for_test(tx, &catalog, &[0, 1], None, None)?;
        let zero_budget = crate::tools::leads::custom::scan_entries_for_test(
            tx,
            &catalog,
            &[0, 1],
            None,
            Some(std::time::Instant::now()),
        )?;
        Ok((end, partial, zero_budget))
    })
    .check("run stopped multi-language scans")?;

    ensure_eq(
        &end,
        &super::scan::ScanEnd::Stopped(super::scan::Stop {
            limited_by: PageLimit::QueryMatchLimit,
            files_scanned: 1,
            files_total: 3,
            covered_languages: HashSet::from(["rust".to_owned()]),
        }),
        "Stop records only the scanned language in covered_languages",
    )?;

    let (counts, completeness) = partial;
    check_partial_custom_scan(&counts, &completeness)?;

    let (zero_counts, zero_completeness) = zero_budget;
    ensure_eq(
        &zero_completeness,
        &Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            resume_after_file: None,
            files_scanned: 0,
            files_total: 3,
        },
        "expired deadline reports 0 files scanned",
    )?;
    for entry in &zero_counts {
        ensure(
            entry.covers.is_empty(),
            format!(
                "zero-files-scanned stop must leave empty covers for {}, got {:?}",
                entry.mechanic, entry.covers
            ),
        )?;
    }
    Ok(())
}
