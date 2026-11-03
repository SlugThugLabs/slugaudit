//! Paginate: what the page context carries and the result-store surface.
use super::*;

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

    ensure_eq(
        &(*seen.borrow()),
        &Some((2048, cols(&["n"]), 2)),
        "*seen.borrow() == Some((2048, cols(&[\"n\"]), 2))",
    )?;
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

    ensure_eq(
        &(*seen.borrow()),
        &Some((2048, cols(&["n"]), 2)),
        "*seen.borrow() == Some((2048, cols(&[\"n\"]), 2))",
    )?;
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

    ensure_eq(
        &(*seen.borrow()),
        &Some((1000, cols(&["a", "b"]), 2)),
        "*seen.borrow() == Some((1000, cols(&[\"a\", \"b\"]), 2))",
    )?;
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
    ensure_eq(
        &store
            .stats()
            .check("read the store counters")?
            .unknown_cursors,
        &0,
        "store .stats() .check(\"read the store counters\")? .unknown_cursors == 0",
    )?;

    let error = next(&manager, "other", &cursor_of(&first)?, 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    ensure_eq(
        &store
            .stats()
            .check("read the store counters")?
            .unknown_cursors,
        &1,
        "store .stats() .check(\"read the store counters\")? .unknown_cursors == 1",
    )?;
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
    ensure_eq(
        &label,
        &crate::util::redacted_path(&root),
        "label == crate::util::redacted_path(&root)",
    )?;
    Ok(())
}

#[test]
fn result_store_surface_verbose_health_reports_the_store_stats() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    ensure(
        next(&manager, "other", &cursor_of(&first)?, 10).is_err(),
        "next(&manager, \"other\", &cursor_of(&first)?, 10).is_err()",
    )?;
    let request = rmcp::handler::server::wrapper::Parameters(crate::tools::HealthRequest {
        path: Some(path.clone()),
        verbose: true,
    });

    let answer = crate::tools::health(&request, &NoopProgressSink, &manager).check("health")?;

    let value = serde_json::to_value(answer.0).check("health serializes")?;
    let store = value
        .get("detail")
        .check("value[\"detail\"][\"result_store\"]")?
        .get("result_store")
        .check("value[\"detail\"][\"result_store\"]")?;
    ensure_eq(
        store
            .get("active")
            .check("store[\"active\"][\"results\"]")?
            .get("results")
            .check("store[\"active\"][\"results\"]")?,
        &1,
        "store[\"active\"][\"results\"] == 1",
    )?;
    ensure(
        store
            .get("active")
            .check("store[\"active\"][\"bytes\"]")?
            .get("bytes")
            .check("store[\"active\"][\"bytes\"]")?
            .as_u64()
            .check("bytes")?
            > 0,
        "store[\"active\"][\"bytes\"].as_u64().check(\"bytes\")? > 0",
    )?;
    ensure_eq(
        store
            .get("delivered")
            .check("store[\"delivered\"][\"results\"]")?
            .get("results")
            .check("store[\"delivered\"][\"results\"]")?,
        &0,
        "store[\"delivered\"][\"results\"] == 0",
    )?;
    ensure_eq(
        &store["budget_bytes"],
        &ROOMY,
        "store[\"budget_bytes\"] == ROOMY",
    )?;
    ensure_eq(
        &store["unknown_cursors"],
        &1,
        "store[\"unknown_cursors\"] == 1",
    )?;
    ensure_eq(
        store
            .get("sweep")
            .check("store[\"sweep\"][\"errors\"]")?
            .get("errors")
            .check("store[\"sweep\"][\"errors\"]")?,
        &0,
        "store[\"sweep\"][\"errors\"] == 0",
    )?;
    ensure(
        store
            .get("sweep")
            .check("store[\"sweep\"][\"first_error\"]")?
            .get("first_error")
            .check("store[\"sweep\"][\"first_error\"]")?
            .is_null(),
        "store[\"sweep\"][\"first_error\"].is_null()",
    )?;
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
    ensure(
        value.get("detail").is_none(),
        "value.get(\"detail\").is_none()",
    )?;
    Ok(())
}
