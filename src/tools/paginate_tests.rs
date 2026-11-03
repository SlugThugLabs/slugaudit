use super::*;
use crate::progress::NoopProgressSink;
use crate::store::{ResultStoreLimits, StoredRow};
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;
use serde::{Deserialize, Serialize};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq, ensure_ne};

const TOOL: &str = "demo";
const ROOMY: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
struct Demo(i64);

impl Materialize for Demo {
    fn columns() -> &'static [&'static str] {
        &["n"]
    }
    fn to_values(&self) -> Vec<Cell> {
        vec![Cell::Int(self.0)]
    }
    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        match values {
            [Cell::Int(n)] => Ok(Self(*n)),
            _ => Err(ResultStoreError::Invalid {
                reason: "demo row layout",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Totals {
    produced: u64,
}

fn limits(max_bytes: u64) -> ResultStoreLimits {
    ResultStoreLimits {
        max_bytes,
        active_grace_secs: 900,
    }
}

fn manager(max_bytes: u64) -> SourceSyncManager {
    SourceSyncManager::with_result_store_limits(limits(max_bytes))
}

fn project() -> TestResult<(tempfile::TempDir, String)> {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

/// Accepts `rows` rows per page (the first is offered and always taken).
fn per_page(rows: usize) -> impl FnMut(&StoredRow) -> Take {
    let mut remaining = rows;
    move |_| match remaining.checked_sub(1) {
        Some(left) => {
            remaining = left;
            Take::Yes
        }
        None => Take::No,
    }
}

fn spec(tool: &'static str) -> PageSpec {
    PageSpec::typed::<Demo>(tool, 1000)
}

fn fresh(
    manager: &SourceSyncManager,
    path: &str,
    rows: i64,
    page_rows: usize,
) -> Result<Served<Totals>, ErrorData> {
    paginate(
        manager,
        &spec(TOOL),
        PageRequest::Fresh {
            path,
            sink: &NoopProgressSink,
        },
        |tx, sink| {
            let one: i64 = tx
                .query_row("SELECT 1", [], |row| row.get(0))
                .map_err(|error| internal_error("test producer", error))?;
            if one != 1 {
                return Err(ErrorData::internal_error(
                    format!("SELECT 1 returned {one}, expected 1"),
                    None,
                ));
            }
            for n in 0..rows {
                sink.push(&Demo(n))?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: Totals {
                    produced: u64::try_from(rows).map_err(|error| {
                        ErrorData::internal_error(format!("rows are non-negative: {error}"), None)
                    })?,
                },
            })
        },
        |_| per_page(page_rows),
    )
}

fn next(
    manager: &SourceSyncManager,
    tool: &'static str,
    token: &str,
    page_rows: usize,
) -> Result<Served<Totals>, ErrorData> {
    paginate(
        manager,
        &spec(tool),
        PageRequest::Cursor(token),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        |_| per_page(page_rows),
    )
}

fn numbers(served: &Served<Totals>) -> TestResult<Vec<i64>> {
    Ok(decode_rows::<Demo>(&served.rows)
        .check("demo rows decode")?
        .into_iter()
        .map(|Demo(n)| n)
        .collect())
}

fn code_of(error: &ErrorData) -> TestResult<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data["code"].as_str())
        .check("the error carries a code")
}

fn cursor_of(served: &Served<Totals>) -> TestResult<String> {
    served
        .page
        .cursor
        .clone()
        .check("more rows remain, so there is a cursor")
}

#[test]
fn paginate_without_a_cursor_syncs_once_and_serves_page_one() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let served = fresh(&manager, &path, 25, 10).check("page one")?;

    ensure(
        manager.last_sync_unix_seconds() > 0,
        "manager.last_sync_unix_seconds() > 0",
    )?;
    ensure_eq(
        &numbers(&served)?,
        &((0..10).collect::<Vec<_>>()),
        "numbers(&served)? == (0..10).collect::<Vec<_>>()",
    )?;
    ensure_eq(&served.page.returned, &10, "served.page.returned == 10")?;
    ensure_eq(
        &served.page.total,
        &Some(25),
        "served.page.total == Some(25)",
    )?;
    ensure(!served.page.complete, "!served.page.complete")?;
    ensure_eq(
        &served.page.limited_by,
        &PageLimit::PageBytes,
        "served.page.limited_by == PageLimit::PageBytes",
    )?;
    ensure_eq(
        &served.page.completeness,
        &Some(Completeness::Complete),
        "served.page.completeness == Some(Completeness::Complete)",
    )?;
    served
        .page
        .cursor
        .as_ref()
        .check("`served.page.cursor` is present")?;
    ensure_eq(
        &served.meta,
        &Totals { produced: 25 },
        "served.meta == Totals { produced: 25 }",
    )?;
    Ok(())
}

#[test]
fn paginate_continuations_cover_the_tail_without_syncing() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    let full = fresh(&manager, &path, 25, usize::MAX).check("a full fetch")?;

    let second = next(&manager, TOOL, &cursor_of(&first)?, 10).check("page two")?;
    let third = next(&manager, TOOL, &cursor_of(&second)?, 10).check("page three")?;

    ensure_eq(
        &second.page.revision_id,
        &first.page.revision_id,
        "second.page.revision_id == first.page.revision_id",
    )?;
    ensure_eq(
        &third.page.revision_id,
        &first.page.revision_id,
        "third.page.revision_id == first.page.revision_id",
    )?;
    let tail: Vec<i64> = numbers(&second)?
        .into_iter()
        .chain(numbers(&third)?)
        .collect();
    ensure_eq(
        &tail,
        &numbers(&full)?
            .get(10..)
            .check("numbers(&full)?[10..]")?
            .to_vec(),
        "tail == numbers(&full)?[10..].to_vec()",
    )?;
    ensure_eq(&third.page.cursor, &None, "third.page.cursor == None")?;
    ensure(third.page.complete, "third.page.complete")?;
    ensure_eq(
        &third.page.limited_by,
        &PageLimit::None,
        "third.page.limited_by == PageLimit::None",
    )?;
    ensure_eq(&third.page.total, &Some(25), "third.page.total == Some(25)")?;
    ensure_eq(
        &third.meta,
        &Totals { produced: 25 },
        "third.meta == Totals { produced: 25 }",
    )?;
    Ok(())
}

#[test]
fn paginate_continuation_ignores_a_revision_published_since_page_one() -> TestResult {
    let (project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    std::fs::write(project.path().join("added.rs"), b"pub fn added() {}\n").check("edit tree")?;
    let moved = crate::sync::test_support::sync_project(&manager, &project)?;
    ensure_ne(
        &moved.revision_id,
        &first.page.revision_id,
        "moved.revision_id != first.page.revision_id",
    )?;

    let second = next(&manager, TOOL, &cursor_of(&first)?, 10).check("page two")?;

    ensure_eq(
        &numbers(&second)?,
        &((10..20).collect::<Vec<_>>()),
        "numbers(&second)? == (10..20).collect::<Vec<_>>()",
    )?;
    ensure_eq(
        &second.page.revision_id,
        &first.page.revision_id,
        "second.page.revision_id == first.page.revision_id",
    )?;
    Ok(())
}

#[test]
fn paginate_same_cursor_twice_returns_the_identical_page() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    let token = cursor_of(&first)?;

    let once = next(&manager, TOOL, &token, 10).check("first read")?;
    let again = next(&manager, TOOL, &token, 10).check("repeat read")?;

    ensure_eq(&once.rows, &again.rows, "once.rows == again.rows")?;
    ensure_eq(&once.page, &again.page, "once.page == again.page")?;
    Ok(())
}

#[test]
fn paginate_final_page_retry_still_works_after_delivery() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 15, 10).check("page one")?;
    let token = cursor_of(&first)?;

    let last = next(&manager, TOOL, &token, 10).check("final page")?;
    let retry = next(&manager, TOOL, &token, 10).check("final page retried")?;

    ensure(last.page.complete, "last.page.complete")?;
    ensure_eq(
        &numbers(&retry)?,
        &((10..15).collect::<Vec<_>>()),
        "numbers(&retry)? == (10..15).collect::<Vec<_>>()",
    )?;
    ensure_eq(&last.rows, &retry.rows, "last.rows == retry.rows")?;
    Ok(())
}

#[test]
fn paginate_cursor_of_another_tool_is_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;

    let error = next(&manager, "other", &cursor_of(&first)?, 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_wrong_tool_cursor_on_the_last_page_does_not_mark_delivered() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 15, 10).check("page one")?;
    let stats = || -> TestResult<_> {
        let stats = manager
            .result_stores()
            .current()
            .check("the store slot")?
            .check("a store")?
            .stats()
            .check("read the store counters")?;
        Ok((
            stats.active.results,
            stats.idle.results,
            stats.delivered.results,
        ))
    };
    let before = stats()?;

    let error = next(&manager, "other", &cursor_of(&first)?, 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    ensure_eq(&before, &(1, 0, 0), "before == (1, 0, 0)")?;
    ensure_eq(&stats()?, &before, "stats()? == before")?;
    next(&manager, TOOL, &cursor_of(&first)?, 10).check("the right tool finishes it")?;
    ensure_eq(&stats()?, &(0, 0, 1), "stats()? == (0, 0, 1)")?;
    Ok(())
}

#[test]
fn paginate_garbage_cursor_is_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    fresh(&manager, &path, 5, 10).check("opens the store")?;

    let error = next(&manager, TOOL, "not-a-cursor", 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_cursor_from_another_process_nonce_is_unknown_cursor() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).check("page one")?;
    let token = cursor_of(&first)?;
    let mut parts: Vec<&str> = token.split('.').collect();
    let foreign = uuid::Uuid::new_v4().hyphenated().to_string();
    *parts.get_mut(1).check("parts[1]")? = &foreign;

    let error = next(&manager, TOOL, &parts.join("."), 10)
        .err()
        .check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_cursor_before_any_result_exists_is_unknown_cursor() -> TestResult {
    let manager = manager(ROOMY);
    let token = format!("v1.{0}.{0}.0", uuid::Uuid::new_v4().hyphenated());

    let error = next(&manager, TOOL, &token, 10).err().check("refused")?;

    ensure_eq(
        &code_of(&error)?,
        &"unknown_cursor",
        "code_of(&error)? == \"unknown_cursor\"",
    )?;
    Ok(())
}

#[test]
fn paginate_producer_failure_mid_stream_leaves_no_cursor_and_no_entry() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let outcome: Result<Served<Totals>, ErrorData> = paginate(
        &manager,
        &spec(TOOL),
        PageRequest::Fresh {
            path: &path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            for n in 0..3 {
                sink.push(&Demo(n))?;
            }
            Err(ErrorData::invalid_params("producer gave up", None))
        },
        |_| per_page(10),
    );

    let error = outcome.err().check("the producer error propagates")?;
    ensure_eq(
        &error.message,
        &"producer gave up",
        "error.message == \"producer gave up\"",
    )?;
    let stats = manager
        .result_stores()
        .current()
        .check("the store slot")?
        .check("store")?
        .stats()
        .check("read the store counters")?;
    ensure_eq(&stats.admitted_bytes, &0, "stats.admitted_bytes == 0")?;
    ensure_eq(
        &(stats.active.results + stats.idle.results + stats.delivered.results),
        &0,
        "stats.active.results + stats.idle.results + stats.delivered.results == 0",
    )?;
    Ok(())
}

#[test]
fn paginate_raw_rows_round_trip_without_a_typed_layout() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let raw = PageSpec {
        tool: "raw",
        columns: vec!["a".to_owned(), "b".to_owned()],
        page_bytes: 10,
        requested_page_bytes: None,
        ceiling: usize::MAX,
    };

    let served: Served<Totals> = paginate(
        &manager,
        &raw,
        PageRequest::Fresh {
            path: &path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            sink.push_cells(&[Cell::Text("x".to_owned()), Cell::Null])?;
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: Totals { produced: 1 },
            })
        },
        |_| per_page(10),
    )
    .check("raw page")?;

    ensure_eq(
        &served.rows.first().check("served.rows[0]")?.cells,
        &vec![Cell::Text("x".to_owned()), Cell::Null],
        "served.rows[0].cells == vec![Cell::Text(\"x\".to_owned()), Cell::Null]",
    )?;
    ensure_eq(&served.page.total, &Some(1), "served.page.total == Some(1)")?;
    Ok(())
}

#[test]
fn page_request_with_a_cursor_continues_and_ignores_the_path() -> TestResult {
    let request = page_request(Some("tok"), Some("/ignored"), &NoopProgressSink)
        .check("a cursor resolves")?;

    ensure(
        matches!(request, PageRequest::Cursor("tok")),
        "matches!(request, PageRequest::Cursor(\"tok\"))",
    )?;
    Ok(())
}

#[test]
fn page_request_without_a_cursor_is_fresh_on_the_path() -> TestResult {
    let request =
        page_request(None, Some("/project"), &NoopProgressSink).check("a path resolves")?;

    ensure(
        matches!(
            request,
            PageRequest::Fresh {
                path: "/project",
                ..
            }
        ),
        "matches!( request, PageRequest::Fresh { path: \"/project\", .. } )",
    )?;
    Ok(())
}

#[test]
fn page_request_with_neither_cursor_nor_path_is_invalid_params() -> TestResult {
    let error = page_request(None, None, &NoopProgressSink)
        .err()
        .check("nothing to resolve")?;

    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure_eq(
        &error.message,
        &"path is required when no cursor is given",
        "error.message == \"path is required when no cursor is given\"",
    )?;
    Ok(())
}

#[path = "paginate_late_tests.rs"]
mod late;

#[path = "paginate_store_limit_tests.rs"]
mod store_limit_tests;
