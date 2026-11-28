use super::*;
pub(super) use crate::progress::NoopProgressSink;
pub(super) use crate::store::{Cell, ResultStoreError, ResultStoreLimits, StoredRow};
pub(super) use crate::sync::SourceSyncManager;
use crate::tools::page::PageLimit;
use crate::tools::test_support::activated_project;
use serde::{Deserialize, Serialize};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq, ensure_ne};

pub(super) const TOOL: &str = "demo";
pub(super) const ROOMY: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Demo(pub(super) i64);

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
pub(super) struct Totals {
    pub(super) produced: u64,
}

fn limits(max_bytes: u64) -> ResultStoreLimits {
    ResultStoreLimits {
        max_bytes,
        active_grace_secs: 900,
    }
}

pub(super) fn manager(max_bytes: u64) -> SourceSyncManager {
    SourceSyncManager::with_result_store_limits(limits(max_bytes))
}

pub(super) fn project() -> TestResult<(tempfile::TempDir, String)> {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let path = project.path().to_string_lossy().into_owned();
    Ok((project, path))
}

/// Single-decode row cut keeping at most `page_rows` `Demo` items per page.
pub(super) fn cut_demo_rows(
    page_rows: usize,
) -> impl FnMut(&StoredRow, usize, usize) -> Result<CutStep<Demo>, ErrorData> {
    move |row, taken, _bytes_so_far| {
        if taken > 0 && taken >= page_rows {
            return Ok(CutStep::Stop { limit: None });
        }
        let demo = Demo::from_values(&row.cells).map_err(|e| store_error(&e))?;
        Ok(CutStep::Keep {
            row: demo,
            bytes: 8,
        })
    }
}

pub(super) fn spec(tool: &'static str) -> PageSpec {
    PageSpec::typed::<Demo>(tool, 1000)
}

pub(super) fn fresh(
    manager: &SourceSyncManager,
    path: &str,
    rows: i64,
    page_rows: usize,
) -> Result<ServedRows<Demo, Totals>, ErrorData> {
    paginate_rows(
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
        |_| Ok(cut_demo_rows(page_rows)),
    )
}

pub(super) fn next(
    manager: &SourceSyncManager,
    tool: &'static str,
    token: &str,
    page_rows: usize,
) -> Result<ServedRows<Demo, Totals>, ErrorData> {
    paginate_rows(
        manager,
        &spec(tool),
        PageRequest::Cursor(token),
        |_, _| {
            Err(ErrorData::internal_error(
                "a continuation never produces",
                None,
            ))
        },
        |_| Ok(cut_demo_rows(page_rows)),
    )
}

pub(super) fn numbers(served: &ServedRows<Demo, Totals>) -> Vec<i64> {
    served.rows.iter().map(|Demo(n)| *n).collect()
}

pub(super) fn code_of(error: &ErrorData) -> TestResult<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data["code"].as_str())
        .check("the error carries a code")
}

pub(super) fn cursor_of<R>(served: &ServedRows<R, Totals>) -> TestResult<String> {
    served
        .page
        .cursor
        .clone()
        .check("more rows remain, so there is a cursor")
}

#[test]
fn limited_by_names_the_cause_for_every_combination() -> TestResult {
    for stopped_by in [
        None,
        Some(PageLimit::Limit),
        Some(PageLimit::MatchCap),
        Some(PageLimit::PageBytes),
    ] {
        for at_ceiling in [false, true] {
            ensure_eq(
                &compute_limited_by(true, stopped_by, at_ceiling),
                &PageLimit::None,
                "complete pages always report PageLimit::None",
            )?;
        }
    }
    for (cap, expected) in [
        (PageLimit::Limit, PageLimit::Limit),
        (PageLimit::MatchCap, PageLimit::MatchCap),
    ] {
        for at_ceiling in [false, true] {
            ensure_eq(
                &compute_limited_by(false, Some(cap), at_ceiling),
                &expected,
                "non-byte cap takes precedence over byte ceiling flag",
            )?;
        }
    }
    for stopped_by in [None, Some(PageLimit::PageBytes)] {
        ensure_eq(
            &compute_limited_by(false, stopped_by, false),
            &PageLimit::PageBytes,
            "byte cut below ceiling reports PageBytes",
        )?;
        ensure_eq(
            &compute_limited_by(false, stopped_by, true),
            &PageLimit::Ceiling,
            "byte cut at ceiling reports Ceiling",
        )?;
    }
    Ok(())
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
        &numbers(&served),
        &((0..10).collect::<Vec<_>>()),
        "numbers(&served) == (0..10)",
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
    let tail: Vec<i64> = numbers(&second)
        .into_iter()
        .chain(numbers(&third))
        .collect();
    ensure_eq(
        &tail,
        &numbers(&full)
            .get(10..)
            .check("numbers(&full)[10..]")?
            .to_vec(),
        "tail == numbers(&full)[10..]",
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
fn paginate_spec_max_rows_cuts_page_one_and_continuations_with_match_cap() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let mut capped_spec = spec(TOOL);
    capped_spec.max_rows = Some(3);

    let first = paginate_rows(
        &manager,
        &capped_spec,
        PageRequest::Fresh {
            path: &path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            for n in 0..7 {
                sink.push(&Demo(n))?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: Totals { produced: 7 },
            })
        },
        |ctx| {
            let budget = ctx.page_bytes;
            Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                let demo = Demo::from_values(&row.cells).map_err(|e| store_error(&e))?;
                Ok(byte_cut_step(demo, 8, taken, bytes_so_far, budget))
            })
        },
    )
    .check("page one")?;

    ensure_eq(&numbers(&first), &[0, 1, 2], "page 1 cut at max_rows=3")?;
    ensure_eq(
        &first.page.limited_by,
        &PageLimit::MatchCap,
        "page 1 limited_by is MatchCap",
    )?;
    ensure(!first.page.complete, "page 1 not complete")?;

    let read_next = |token: &str| {
        paginate_rows(
            &manager,
            &spec(TOOL),
            PageRequest::Cursor(token),
            |_, _| Err(ErrorData::internal_error("unexpected produce", None)),
            |ctx| {
                let budget = ctx.page_bytes;
                Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                    let demo = Demo::from_values(&row.cells).map_err(|e| store_error(&e))?;
                    Ok(byte_cut_step(demo, 8, taken, bytes_so_far, budget))
                })
            },
        )
    };

    let second = read_next(&cursor_of(&first)?).check("page two")?;
    ensure_eq(&numbers(&second), &[3, 4, 5], "stored max_rows cuts page 2")?;
    ensure_eq(
        &second.page.limited_by,
        &PageLimit::MatchCap,
        "page 2 limited_by is MatchCap",
    )?;

    let third = read_next(&cursor_of(&second)?).check("page three")?;
    ensure_eq(&numbers(&third), &[6], "page 3 finishes cleanly")?;
    ensure(third.page.complete, "page 3 is complete")?;
    ensure_eq(
        &third.page.limited_by,
        &PageLimit::None,
        "final page limited_by is None",
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
        &numbers(&second),
        &((10..20).collect::<Vec<_>>()),
        "numbers(&second) == (10..20)",
    )?;
    ensure_eq(
        &second.page.revision_id,
        &first.page.revision_id,
        "second.page.revision_id == first.page.revision_id",
    )?;
    ensure_eq(
        &first.page.current_disk_revision_id,
        &None,
        "fresh page one omits current_disk_revision_id",
    )?;
    ensure_eq(
        &second.page.current_disk_revision_id,
        &Some(moved.revision_id),
        "continuation reports the newer revision currently on disk",
    )?;
    Ok(())
}

#[test]
fn paginate_same_cursor_twice_and_final_page_retry_return_identical_pages() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 15, 10).check("page one")?;
    let token = cursor_of(&first)?;

    let last = next(&manager, TOOL, &token, 10).check("final page")?;
    let retry = next(&manager, TOOL, &token, 10).check("final page retried")?;

    ensure(last.page.complete, "last.page.complete")?;
    ensure_eq(&numbers(&retry), &((10..15).collect::<Vec<_>>()), "10..15")?;
    ensure_eq(&last.rows, &retry.rows, "last.rows == retry.rows")?;
    ensure_eq(&last.page, &retry.page, "last.page == retry.page")?;
    Ok(())
}

#[test]
fn paginate_producer_failure_mid_stream_leaves_no_cursor_and_no_entry() -> TestResult {
    let (_project, path) = project()?;
    let manager = manager(ROOMY);

    let outcome: Result<ServedRows<Demo, Totals>, ErrorData> = paginate_rows(
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
        |_| Ok(cut_demo_rows(10)),
    );

    let error = outcome.err().check("the producer error propagates")?;
    ensure_eq(
        &error.message,
        &"producer gave up",
        "error.message == \"producer gave up\"",
    )?;
    if let Some(store) = manager.result_stores().current().check("the store slot")? {
        let stats = store.stats().check("read the store counters")?;
        ensure_eq(&stats.admitted_bytes, &0, "stats.admitted_bytes == 0")?;
        ensure_eq(
            &(stats
                .active
                .results
                .checked_add(stats.idle.results)
                .and_then(|n| n.checked_add(stats.delivered.results))
                .check("results sum")?),
            &0,
            "no active, idle, or delivered entries exist",
        )?;
    }
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
        max_rows: None,
        discard_on_partial: false,
        store_limits: None,
    };

    let served: ServedRows<Vec<Cell>, Totals> = paginate_rows(
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
        |_| {
            Ok(|row: &StoredRow, _taken: usize, _bytes: usize| {
                Ok(CutStep::Keep {
                    row: row.cells.clone(),
                    bytes: 4,
                })
            })
        },
    )
    .check("raw page")?;

    ensure_eq(
        served.rows.first().check("served.rows[0]")?,
        &vec![Cell::Text("x".to_owned()), Cell::Null],
        "raw cells round-trip cleanly",
    )?;
    ensure_eq(&served.page.total, &Some(1), "served.page.total == Some(1)")?;
    Ok(())
}
