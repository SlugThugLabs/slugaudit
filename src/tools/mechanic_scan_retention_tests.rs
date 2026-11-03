//! Registered-entry scans: the retained budget, and out-of-range entry indexes.
use super::*;

#[test]
fn a_pass_over_many_entries_retains_at_most_the_budget_in_total() -> TestResult {
    use crate::tools::mechanic_scan_cached::Pass;
    let row = crate::project::CachedRow {
        file: "a.py".to_owned(),
        line: 1,
        text: "x".to_owned(),
    };
    let size = row.estimated_bytes();
    let mut pass = Pass::new(size * 5, 4);

    for _ in 0..3 {
        for entry in 0..4 {
            pass.offer(entry, row.clone()).check("offer a row")?;
            ensure(
                pass.retained_bytes() <= size * 5,
                format!("retained {} past the shared budget", pass.retained_bytes()),
            )?;
        }
    }

    ensure(
        pass.retained_bytes() <= size * 5,
        "pass.retained_bytes() <= size * 5",
    )?;
    Ok(())
}

/// A selected index that names no slot is an internal error, never a silently
/// dropped row or count.
#[test]
fn an_entry_index_out_of_range_fails_the_pass_and_the_counts() -> TestResult {
    use crate::tools::mechanic_scan_cached::Pass;
    let row = crate::project::CachedRow {
        file: "a.py".to_owned(),
        line: 1,
        text: "x".to_owned(),
    };
    let mut pass = Pass::new(1024, 2);

    let offered = pass.offer(2, row).check_err("index 2 of 2 entries")?;
    let counted = crate::tools::leads_custom::counts(
        &mechanic_catalog::Catalog {
            custom: Vec::new(),
            invalid: Vec::new(),
        },
        &[0],
        &[],
        &crate::tools::leads_covers::LanguagesRead::empty(),
    )
    .check_err("index 0 of an empty catalogue")?;

    for error in [offered, counted] {
        ensure_eq(
            &error.code,
            &rmcp::model::ErrorCode::INTERNAL_ERROR,
            "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
        )?;
        ensure(
            error.message.contains("out of range"),
            format!("{}", error.message),
        )?;
    }
    Ok(())
}

#[test]
fn many_uncached_entries_in_one_call_over_a_tiny_budget_are_all_counted_exactly() -> TestResult {
    let sizing = sync::SourceSyncManager::default();
    let project = python_project()?;
    let path = path_of(&project)?;
    call(&sizing, add(&path, "sizing", "python", EVAL_QUERY)).check("add")?;
    let one_entry = cache_stats(&sizing)?.bytes;
    let budget = one_entry + one_entry / 2;
    drop(sizing);
    let manager = sync::SourceSyncManager::with_mechanic_cache_budget(budget);
    for id in ["e1", "e2", "e3", "e4", "e5"] {
        call(&manager, add(&path, id, "python", EVAL_QUERY)).check("add")?;
    }
    std::fs::write(
        project.path().join("extra.py"),
        "eval(\"4\")\neval(\"5\")\n",
    )
    .check("new revision")?;
    let skipped_before = cache_stats(&manager)?.skipped_too_large;

    let response = run_leads(&manager, leads_request(&path, None, LeadMode::Full))?;

    for id in ["e1", "e2", "e3", "e4", "e5"] {
        ensure_eq(
            &count_of(&response, id)?,
            &6,
            "count_of(&response, id)? == 6",
        )?;
    }
    let stats = cache_stats(&manager)?;
    ensure(stats.bytes <= budget, format!("{} > {budget}", stats.bytes))?;
    ensure(
        stats.skipped_too_large > skipped_before,
        "stats.skipped_too_large > skipped_before",
    )?;
    Ok(())
}

#[test]
fn a_selected_index_outside_the_catalogue_fails_the_call_instead_of_being_skipped() -> TestResult {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;
    let tx = connection.unchecked_transaction().check("transaction")?;
    let cache = crate::project::MechanicScanCache::with_budget(1024);
    let scope = crate::tools::mechanic_scan_cached::CacheScope {
        cache: &cache,
        project: "p",
        revision: "r",
    };

    let Err(error) =
        crate::tools::mechanic_scan_cached::tally_cached(&tx, &[], &[0], None, None, &scope)
    else {
        return Err("an out-of-range index must fail the call".to_owned())
            .check("unexpected outcome");
    };

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure(
        error.message.starts_with(
            "counting mechanics: entry index 0 is out of range for custom of length 0.",
        ),
        format!("{}", error.message),
    )?;
    Ok(())
}
