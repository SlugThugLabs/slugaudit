//! Columns declared by the producer, the stored page budget and the offered
//! first row of `paginate`.

use super::*;
use slugaudit_test_check::{Check, TestResult};

/// A spec with no columns of its own, for producers that declare them.
fn late_spec(tool: &'static str) -> PageSpec {
    PageSpec {
        tool,
        columns: Vec::new(),
        page_bytes: 1000,
        requested_page_bytes: None,
        ceiling: usize::MAX,
    }
}

fn produced_totals() -> Produced<Totals> {
    Produced {
        completeness: Completeness::Complete,
        extra: Totals { produced: 2 },
    }
}

fn cols(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn late_fresh(manager: &SourceSyncManager, path: &str) -> Result<Served<Totals>, ErrorData> {
    paginate(
        manager,
        &late_spec("late"),
        PageRequest::Fresh {
            path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            sink.set_columns(cols(&["a", "b"]))?;
            sink.push_cells(&[Cell::Int(1), Cell::Null])?;
            sink.push_cells(&[Cell::Int(2), Cell::Null])?;
            Ok(produced_totals())
        },
        |_| per_page(1),
    )
}

#[test]
fn paginate_columns_set_by_the_producer_are_stored_and_returned_on_every_page() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let first = late_fresh(&manager, &path).check("page one")?;
    let second: Served<Totals> = paginate(
        &manager,
        &late_spec("late"),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        |_| per_page(1),
    )
    .check("page two")?;

    assert_eq!(first.columns, cols(&["a", "b"]));
    assert_eq!(second.columns, cols(&["a", "b"]));
    assert_eq!(second.rows[0].cells, vec![Cell::Int(2), Cell::Null]);
    Ok(())
}

#[test]
fn paginate_typed_spec_columns_are_returned_without_a_set_columns_call() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let served = fresh(&manager, &path, 3, 10).check("page")?;

    assert_eq!(served.columns, cols(&["n"]));
    Ok(())
}

fn declare_twice_or_late(
    manager: &SourceSyncManager,
    path: &str,
    spec: &PageSpec,
    misuse: impl FnOnce(&mut RowSink<'_, '_>) -> Result<(), ErrorData>,
) -> TestResult<ErrorData> {
    paginate::<Totals, _, _>(
        manager,
        spec,
        PageRequest::Fresh {
            path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            sink.set_columns(cols(&["a"]))?;
            misuse(sink)?;
            Ok(produced_totals())
        },
        |_| per_page(1),
    )
    .err()
    .check("the misuse fails the call")
}

#[test]
fn paginate_set_columns_twice_is_an_internal_error_and_leaves_nothing_stored() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let error = declare_twice_or_late(&manager, &path, &late_spec("late"), |sink| {
        sink.set_columns(cols(&["b"]))
    })?;

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error
            .message
            .starts_with("declaring the result columns: the result columns were already declared."),
        "{}",
        error.message
    );
    assert_eq!(
        manager
            .result_stores()
            .current()
            .check("the store slot")?
            .check("a store")?
            .stats()
            .check("the store counters")?
            .admitted_bytes,
        0
    );
    Ok(())
}

#[test]
fn paginate_set_columns_after_a_push_is_an_internal_error() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let error = declare_twice_or_late(&manager, &path, &late_spec("late"), |sink| {
        sink.push_cells(&[Cell::Int(1)])?;
        sink.set_columns(cols(&["b"]))
    })?;

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("declaring the result columns: "),
        "{}",
        error.message
    );
    Ok(())
}

#[test]
fn paginate_set_columns_when_the_spec_has_columns_is_an_internal_error() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let error = paginate::<Totals, _, _>(
        &manager,
        &spec("typed"),
        PageRequest::Fresh {
            path: &path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            sink.set_columns(cols(&["b"]))?;
            Ok(produced_totals())
        },
        |_| per_page(1),
    )
    .err()
    .check("columns are already known")?;

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    Ok(())
}

#[test]
fn paginate_a_push_before_the_columns_are_declared_is_an_internal_error() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let error = paginate::<Totals, _, _>(
        &manager,
        &late_spec("late"),
        PageRequest::Fresh {
            path: &path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            sink.push_cells(&[Cell::Int(1)])?;
            Ok(produced_totals())
        },
        |_| per_page(1),
    )
    .err()
    .check("no columns yet")?;

    assert!(
        error.message.starts_with(
            "storing a result row: a row was pushed before the result columns were declared."
        ),
        "{}",
        error.message
    );
    Ok(())
}

/// A budget-recording accept closure for `paginate`.
fn budget_probe(
    seen: &std::cell::Cell<usize>,
) -> impl FnOnce(&PageContext<'_, Totals>) -> fn(&StoredRow) -> Take {
    |ctx| {
        seen.set(ctx.page_bytes);
        |_| Take::Yes
    }
}

fn budget_spec(page_bytes: usize, requested: Option<usize>) -> PageSpec {
    PageSpec {
        page_bytes,
        requested_page_bytes: requested,
        ..PageSpec::typed::<Demo>(TOOL, page_bytes)
    }
}

fn budgeted_page_one(
    manager: &SourceSyncManager,
    path: &str,
    budget: usize,
) -> TestResult<Served<Totals>> {
    budgeted_page_one_under(manager, path, budget, usize::MAX)
}

fn budgeted_page_one_under(
    manager: &SourceSyncManager,
    path: &str,
    budget: usize,
    ceiling: usize,
) -> TestResult<Served<Totals>> {
    let seen = std::cell::Cell::new(0);
    let served = paginate(
        manager,
        &PageSpec {
            ceiling,
            ..budget_spec(budget, None)
        },
        PageRequest::Fresh {
            path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            for n in 0..3 {
                sink.push(&Demo(n))?;
            }
            Ok(produced_totals())
        },
        |ctx| {
            seen.set(ctx.page_bytes);
            per_page(1)
        },
    )
    .check("page one")?;
    assert_eq!(
        seen.get(),
        budget,
        "page one is cut by the configured budget"
    );
    Ok(served)
}

#[test]
fn paginate_continuation_without_page_bytes_uses_the_budget_stored_with_the_result() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048)?;
    assert_eq!(first.page.page_bytes, 2048);
    let seen = std::cell::Cell::new(0);

    let second: Served<Totals> = paginate(
        &manager,
        &budget_spec(77, None),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        budget_probe(&seen),
    )
    .check("page two")?;

    assert_eq!(seen.get(), 2048);
    assert_eq!(second.page.page_bytes, 2048);
    Ok(())
}

#[test]
fn paginate_continuation_with_an_explicit_page_bytes_uses_and_echoes_it() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048)?;
    let seen = std::cell::Cell::new(0);

    let second: Served<Totals> = paginate(
        &manager,
        &budget_spec(77, Some(512)),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        budget_probe(&seen),
    )
    .check("page two")?;

    assert_eq!(seen.get(), 512);
    assert_eq!(second.page.page_bytes, 512);
    Ok(())
}

/// The ceiling page one ran under is stored with the result: a continuation
/// clamps an over-large `page_bytes` to it, and reports being at it, whatever
/// any other limits say.
#[test]
fn paginate_continuation_page_bytes_above_the_stored_ceiling_is_clamped_to_it() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = budgeted_page_one_under(&manager, &path, 2048, 4096)?;
    assert!(!first.at_ceiling, "2048 is below the ceiling 4096");
    let seen = std::cell::Cell::new(0);

    let second: Served<Totals> = paginate(
        &manager,
        &budget_spec(77, Some(1 << 20)),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        budget_probe(&seen),
    )
    .check("page two")?;

    assert_eq!((seen.get(), second.page.page_bytes), (4096, 4096));
    assert!(second.at_ceiling, "the clamped budget is the ceiling");
    Ok(())
}

#[test]
fn paginate_offers_the_first_row_to_accept_and_still_takes_it() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 5, 2).check("page one")?;
    let asked = std::cell::RefCell::new(Vec::new());

    let second: Served<Totals> = paginate(
        &manager,
        &spec(TOOL),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        |_| {
            |row: &StoredRow| {
                asked.borrow_mut().push(row.ord);
                Take::No
            }
        },
    )
    .check("page two")?;

    assert_eq!(*asked.borrow(), vec![2, 3]);
    assert_eq!(numbers(&second)?, vec![2]);
    Ok(())
}

/// What `accept_for` was handed: budget, columns and the extras' `produced`.
type Seen = std::cell::RefCell<Option<(usize, Vec<String>, u64)>>;

fn context_probe(seen: &Seen) -> impl FnOnce(&PageContext<'_, Totals>) -> fn(&StoredRow) -> Take {
    |ctx| {
        *seen.borrow_mut() = Some((ctx.page_bytes, ctx.columns.to_vec(), ctx.extra.produced));
        |_| Take::Yes
    }
}

#[test]
fn paginate_context_on_a_new_request_carries_the_produced_extras_and_columns() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let seen: Seen = Seen::default();

    paginate(
        &manager,
        &budget_spec(2048, None),
        PageRequest::Fresh {
            path: &path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            sink.push(&Demo(1))?;
            Ok(produced_totals())
        },
        context_probe(&seen),
    )
    .check("page one")?;

    assert_eq!(*seen.borrow(), Some((2048, cols(&["n"]), 2)));
    Ok(())
}

#[test]
fn paginate_context_on_a_continuation_carries_the_extras_and_columns_of_page_one() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048)?;
    let seen: Seen = Seen::default();

    paginate(
        &manager,
        &budget_spec(77, None),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        context_probe(&seen),
    )
    .check("page two")?;

    assert_eq!(*seen.borrow(), Some((2048, cols(&["n"]), 2)));
    Ok(())
}

#[test]
fn paginate_context_columns_declared_by_the_producer_reach_the_continuation() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = late_fresh(&manager, &path).check("page one")?;
    let seen: Seen = Seen::default();

    paginate(
        &manager,
        &late_spec("late"),
        PageRequest::Cursor(&cursor_of(&first)?),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        context_probe(&seen),
    )
    .check("page two")?;

    assert_eq!(*seen.borrow(), Some((1000, cols(&["a", "b"]), 2)));
    Ok(())
}

#[test]
fn result_store_surface_wrong_tool_cursor_counts_as_an_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    let store = manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("a store")?;
    assert_eq!(
        store
            .stats()
            .check("read the store counters")?
            .unknown_cursors,
        0
    );

    let error = next(&manager, "other", &cursor_of(&first)?, 10)
        .err()
        .check("refused")?;

    assert_eq!(code_of(&error)?, "unknown_cursor");
    assert_eq!(
        store
            .stats()
            .check("read the store counters")?
            .unknown_cursors,
        1
    );
    Ok(())
}

#[test]
fn result_store_surface_stored_project_label_is_the_root_path_id_of_the_logs() -> TestResult {
    let (project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    let store = manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("a store")?;
    let cursor = store
        .decode_cursor(&cursor_of(&first)?)
        .check("the cursor decodes")?;

    let label = store.meta_of(cursor.result).check("meta")?.project.clone();

    let root = std::fs::canonicalize(project.path()).check("canonical root")?;
    assert_eq!(label, crate::util::redacted_path(&root));
    Ok(())
}

#[test]
fn result_store_surface_verbose_health_reports_the_store_stats() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    assert!(next(&manager, "other", &cursor_of(&first)?, 10).is_err());
    let request = rmcp::handler::server::wrapper::Parameters(crate::tools::HealthRequest {
        path: Some(path.clone()),
        verbose: true,
    });

    let answer = crate::tools::health(&request, &NoopProgressSink, &manager).check("health")?;

    let value = serde_json::to_value(answer.0).check("health serializes")?;
    let store = &value["detail"]["result_store"];
    assert_eq!(store["active"]["results"], 1);
    assert!(store["active"]["bytes"].as_u64().check("bytes")? > 0);
    assert_eq!(store["delivered"]["results"], 0);
    assert_eq!(store["budget_bytes"], ROOMY);
    assert_eq!(store["unknown_cursors"], 1);
    assert_eq!(store["sweep"]["errors"], 0);
    assert!(store["sweep"]["first_error"].is_null());
    Ok(())
}

#[test]
fn result_store_surface_default_health_omits_the_store_detail() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    fresh(&manager, &path, 25, 10).check("page one")?;
    let request = rmcp::handler::server::wrapper::Parameters(crate::tools::HealthRequest {
        path: Some(path.clone()),
        verbose: false,
    });

    let answer = crate::tools::health(&request, &NoopProgressSink, &manager).check("health")?;

    let value = serde_json::to_value(answer.0).check("health serializes")?;
    assert!(value.get("detail").is_none());
    Ok(())
}
