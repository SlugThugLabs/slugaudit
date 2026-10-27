//! Columns declared by the producer, the stored page budget and the offered
//! first row of `paginate`.

use super::*;

/// A spec with no columns of its own, for producers that declare them.
fn late_spec(tool: &'static str) -> PageSpec {
    PageSpec {
        tool,
        columns: Vec::new(),
        page_bytes: 1000,
        requested_page_bytes: None,
        offer_first_row: false,
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
        per_page(1),
    )
}

#[test]
fn paginate_columns_set_by_the_producer_are_stored_and_returned_on_every_page() {
    let (_project, path) = project();
    let manager = manager(ROOMY);

    let first = late_fresh(&manager, &path).expect("page one");
    let second: Served<Totals> = paginate(
        &manager,
        &late_spec("late"),
        PageRequest::Cursor(&cursor_of(&first)),
        |_, _| panic!("a continuation never produces"),
        per_page(1),
    )
    .expect("page two");

    assert_eq!(first.columns, cols(&["a", "b"]));
    assert_eq!(second.columns, cols(&["a", "b"]));
    assert_eq!(second.rows[0].cells, vec![Cell::Int(2), Cell::Null]);
}

#[test]
fn paginate_typed_spec_columns_are_returned_without_a_set_columns_call() {
    let (_project, path) = project();
    let manager = manager(ROOMY);

    let served = fresh(&manager, &path, 3, 10).expect("page");

    assert_eq!(served.columns, cols(&["n"]));
}

fn declare_twice_or_late(
    manager: &SourceSyncManager,
    path: &str,
    spec: &PageSpec,
    misuse: impl FnOnce(&mut RowSink<'_, '_>) -> Result<(), ErrorData>,
) -> ErrorData {
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
        per_page(1),
    )
    .err()
    .expect("the misuse fails the call")
}

#[test]
fn paginate_set_columns_twice_is_an_internal_error_and_leaves_nothing_stored() {
    let (_project, path) = project();
    let manager = manager(ROOMY);

    let error = declare_twice_or_late(&manager, &path, &late_spec("late"), |sink| {
        sink.set_columns(cols(&["b"]))
    });

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
            .map(|s| s.stats().admitted_bytes),
        Some(0)
    );
}

#[test]
fn paginate_set_columns_after_a_push_is_an_internal_error() {
    let (_project, path) = project();
    let manager = manager(ROOMY);

    let error = declare_twice_or_late(&manager, &path, &late_spec("late"), |sink| {
        sink.push_cells(&[Cell::Int(1)])?;
        sink.set_columns(cols(&["b"]))
    });

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("declaring the result columns: "),
        "{}",
        error.message
    );
}

#[test]
fn paginate_set_columns_when_the_spec_has_columns_is_an_internal_error() {
    let (_project, path) = project();
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
        per_page(1),
    )
    .err()
    .expect("columns are already known");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
}

#[test]
fn paginate_a_push_before_the_columns_are_declared_is_an_internal_error() {
    let (_project, path) = project();
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
        per_page(1),
    )
    .err()
    .expect("no columns yet");

    assert!(
        error.message.starts_with(
            "storing a result row: a row was pushed before the result columns were declared."
        ),
        "{}",
        error.message
    );
}

/// A budget-recording accept closure for `paginate_with`.
fn budget_probe(seen: &std::cell::Cell<usize>) -> impl FnOnce(usize) -> fn(&StoredRow) -> Take {
    |budget| {
        seen.set(budget);
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

fn budgeted_page_one(manager: &SourceSyncManager, path: &str, budget: usize) -> Served<Totals> {
    let seen = std::cell::Cell::new(0);
    let served = paginate_with(
        manager,
        &budget_spec(budget, None),
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
        |budget| {
            seen.set(budget);
            per_page(1)
        },
    )
    .expect("page one");
    assert_eq!(
        seen.get(),
        budget,
        "page one is cut by the configured budget"
    );
    served
}

#[test]
fn paginate_continuation_without_page_bytes_uses_the_budget_stored_with_the_result() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048);
    assert_eq!(first.page.page_bytes, 2048);
    let seen = std::cell::Cell::new(0);

    let second: Served<Totals> = paginate_with(
        &manager,
        &budget_spec(77, None),
        PageRequest::Cursor(&cursor_of(&first)),
        |_, _| panic!("a continuation never produces"),
        budget_probe(&seen),
    )
    .expect("page two");

    assert_eq!(seen.get(), 2048);
    assert_eq!(second.page.page_bytes, 2048);
}

#[test]
fn paginate_continuation_with_an_explicit_page_bytes_uses_and_echoes_it() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048);
    let seen = std::cell::Cell::new(0);

    let second: Served<Totals> = paginate_with(
        &manager,
        &budget_spec(77, Some(512)),
        PageRequest::Cursor(&cursor_of(&first)),
        |_, _| panic!("a continuation never produces"),
        budget_probe(&seen),
    )
    .expect("page two");

    assert_eq!(seen.get(), 512);
    assert_eq!(second.page.page_bytes, 512);
}

#[test]
fn paginate_continuation_page_bytes_above_the_ceiling_is_clamped_by_the_tool_resolution() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048);
    let limits = crate::model::ResourceLimits {
        max_query_response_bytes: 4096,
        ..crate::model::ResourceLimits::default()
    };
    let size = crate::tools::page_size::resolve(Some(1 << 20), &limits).expect("clamps");
    let seen = std::cell::Cell::new(0);

    let second: Served<Totals> = paginate_with(
        &manager,
        &budget_spec(77, Some(size.bytes)),
        PageRequest::Cursor(&cursor_of(&first)),
        |_, _| panic!("a continuation never produces"),
        budget_probe(&seen),
    )
    .expect("page two");

    assert_eq!((seen.get(), second.page.page_bytes), (4096, 4096));
}

#[test]
fn paginate_legacy_entry_keeps_the_callers_budget_on_a_continuation() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = budgeted_page_one(&manager, &path, 2048);

    let second = next(&manager, TOOL, &cursor_of(&first), 1).expect("page two");

    assert_eq!(second.page.page_bytes, 1000, "the spec's budget, as before");
}

#[test]
fn paginate_offer_first_row_shows_the_first_row_to_accept_and_still_takes_it() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 5, 2).expect("page one");
    let asked = std::cell::RefCell::new(Vec::new());
    let mut offering = spec(TOOL);
    offering.offer_first_row = true;

    let second: Served<Totals> = paginate(
        &manager,
        &offering,
        PageRequest::Cursor(&cursor_of(&first)),
        |_, _| panic!("a continuation never produces"),
        |row| {
            asked.borrow_mut().push(row.ord);
            Take::No
        },
    )
    .expect("page two");

    assert_eq!(*asked.borrow(), vec![2, 3]);
    assert_eq!(numbers(&second), vec![2]);
}
