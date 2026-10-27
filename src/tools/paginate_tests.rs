use super::*;
use crate::progress::NoopProgressSink;
use crate::store::{ResultStoreLimits, StoredRow};
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;
use serde::{Deserialize, Serialize};

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

fn project() -> (tempfile::TempDir, String) {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    (project, path)
}

/// Accepts `rows` rows per page (the first is always taken by the store).
fn per_page(rows: usize) -> impl FnMut(&StoredRow) -> Take {
    let mut seen = 1;
    move |_| {
        seen += 1;
        if seen <= rows { Take::Yes } else { Take::No }
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
            assert_eq!(one, 1, "the producer runs inside the read transaction");
            for n in 0..rows {
                sink.push(&Demo(n))?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: Totals {
                    produced: u64::try_from(rows).expect("rows are non-negative"),
                },
            })
        },
        per_page(page_rows),
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
        |_, _| panic!("a continuation never produces"),
        per_page(page_rows),
    )
}

fn numbers(served: &Served<Totals>) -> Vec<i64> {
    decode_rows::<Demo>(&served.rows)
        .expect("demo rows decode")
        .into_iter()
        .map(|Demo(n)| n)
        .collect()
}

fn code_of(error: &ErrorData) -> &str {
    error
        .data
        .as_ref()
        .and_then(|data| data["code"].as_str())
        .expect("the error carries a code")
}

fn cursor_of(served: &Served<Totals>) -> String {
    served
        .page
        .cursor
        .clone()
        .expect("more rows remain, so there is a cursor")
}

#[test]
fn paginate_without_a_cursor_syncs_once_and_serves_page_one() {
    let (_project, path) = project();
    let manager = manager(ROOMY);

    let served = fresh(&manager, &path, 25, 10).expect("page one");

    assert_eq!(manager.consecutive_full_publishes(), 1, "exactly one sync");
    assert_eq!(numbers(&served), (0..10).collect::<Vec<_>>());
    assert_eq!(served.page.returned, 10);
    assert_eq!(served.page.total, Some(25));
    assert!(!served.page.complete);
    assert_eq!(served.page.limited_by, PageLimit::PageBytes);
    assert_eq!(served.page.completeness, Some(Completeness::Complete));
    assert_eq!(served.page.next_offset, None);
    assert!(served.page.cursor.is_some());
    assert_eq!(served.meta, Totals { produced: 25 });
}

#[test]
fn paginate_continuations_cover_the_tail_without_syncing() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).expect("page one");
    let full = fresh(&manager, &path, 25, usize::MAX).expect("a full fetch");
    let syncs = manager.consecutive_full_publishes();

    let second = next(&manager, TOOL, &cursor_of(&first), 10).expect("page two");
    let third = next(&manager, TOOL, &cursor_of(&second), 10).expect("page three");

    assert_eq!(manager.consecutive_full_publishes(), syncs, "no sync");
    let tail: Vec<i64> = numbers(&second)
        .into_iter()
        .chain(numbers(&third))
        .collect();
    assert_eq!(tail, numbers(&full)[10..].to_vec());
    assert_eq!(third.page.cursor, None);
    assert!(third.page.complete);
    assert_eq!(third.page.limited_by, PageLimit::None);
    assert_eq!(third.page.total, Some(25));
    assert_eq!(third.meta, Totals { produced: 25 });
}

#[test]
fn paginate_continuation_ignores_a_revision_published_since_page_one() {
    let (project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).expect("page one");
    std::fs::write(project.path().join("added.rs"), b"pub fn added() {}\n").expect("edit tree");
    let moved = crate::sync::test_support::sync_project(&manager, &project);
    assert_ne!(moved.revision_id, first.page.revision_id, "a new revision");

    let second = next(&manager, TOOL, &cursor_of(&first), 10).expect("page two");

    assert_eq!(numbers(&second), (10..20).collect::<Vec<_>>());
    assert_eq!(second.page.revision_id, first.page.revision_id);
}

#[test]
fn paginate_same_cursor_twice_returns_the_identical_page() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).expect("page one");
    let token = cursor_of(&first);

    let once = next(&manager, TOOL, &token, 10).expect("first read");
    let again = next(&manager, TOOL, &token, 10).expect("repeat read");

    assert_eq!(once.rows, again.rows);
    assert_eq!(once.page, again.page);
}

#[test]
fn paginate_final_page_retry_still_works_after_delivery() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 15, 10).expect("page one");
    let token = cursor_of(&first);

    let last = next(&manager, TOOL, &token, 10).expect("final page");
    let retry = next(&manager, TOOL, &token, 10).expect("final page retried");

    assert!(last.page.complete);
    assert_eq!(numbers(&retry), (10..15).collect::<Vec<_>>());
    assert_eq!(last.rows, retry.rows);
}

#[test]
fn paginate_cursor_of_another_tool_is_unknown_cursor() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).expect("page one");

    let error = next(&manager, "other", &cursor_of(&first), 10)
        .err()
        .expect("refused");

    assert_eq!(code_of(&error), "unknown_cursor");
}

#[test]
fn paginate_garbage_cursor_is_unknown_cursor() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    fresh(&manager, &path, 5, 10).expect("opens the store");

    let error = next(&manager, TOOL, "not-a-cursor", 10)
        .err()
        .expect("refused");

    assert_eq!(code_of(&error), "unknown_cursor");
}

#[test]
fn paginate_cursor_from_another_process_nonce_is_unknown_cursor() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let first = fresh(&manager, &path, 25, 10).expect("page one");
    let token = cursor_of(&first);
    let mut parts: Vec<&str> = token.split('.').collect();
    let foreign = uuid::Uuid::new_v4().hyphenated().to_string();
    parts[1] = &foreign;

    let error = next(&manager, TOOL, &parts.join("."), 10)
        .err()
        .expect("refused");

    assert_eq!(code_of(&error), "unknown_cursor");
}

#[test]
fn paginate_cursor_before_any_result_exists_is_unknown_cursor() {
    let manager = manager(ROOMY);
    let token = format!("v1.{0}.{0}.0", uuid::Uuid::new_v4().hyphenated());

    let error = next(&manager, TOOL, &token, 10).err().expect("refused");

    assert_eq!(code_of(&error), "unknown_cursor");
}

/// Bytes one 20-row demo result takes in the store.
fn one_result_bytes() -> u64 {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    fresh(&manager, &path, 20, 10).expect("probe result");
    manager
        .result_stores()
        .current()
        .expect("store")
        .stats()
        .admitted_bytes
}

#[test]
fn paginate_reclaimed_result_continues_as_result_expired() {
    let one = one_result_bytes();
    let (_project, path) = project();
    let manager = manager(one + one / 2);
    let first = fresh(&manager, &path, 20, 10).expect("page one");
    let stale = cursor_of(&first);
    next(&manager, TOOL, &stale, 10).expect("delivers the result");

    fresh(&manager, &path, 20, 10).expect("a new result reclaims the delivered one");
    let error = next(&manager, TOOL, &stale, 10).err().expect("reclaimed");

    assert_eq!(code_of(&error), "result_expired");
    assert_eq!(
        error.data.as_ref().expect("payload")["remedy"],
        crate::error_codes::ErrorCode::ResultExpired.remedy()
    );
}

#[test]
fn paginate_full_store_refuses_a_new_result_and_keeps_active_ones_readable() {
    let one = one_result_bytes();
    let (_project, path) = project();
    let manager = manager(one + one / 2);
    let first = fresh(&manager, &path, 20, 10).expect("page one");

    let error = fresh(&manager, &path, 20, 10).err().expect("store is full");

    assert_eq!(code_of(&error), "result_store_full");
    assert!(error.message.contains("1 active"), "{}", error.message);
    let second = next(&manager, TOOL, &cursor_of(&first), 10).expect("still readable");
    assert_eq!(numbers(&second), (10..20).collect::<Vec<_>>());
}

#[test]
fn paginate_result_over_the_whole_budget_is_result_too_large_with_no_entry() {
    let (_project, path) = project();
    let manager = manager(4096);

    let error = fresh(&manager, &path, 50, 10).err().expect("too large");

    assert_eq!(code_of(&error), "result_too_large");
    let stats = manager.result_stores().current().expect("store").stats();
    assert_eq!(stats.admitted_bytes, 0);
    assert_eq!(stats.active.results, 0);
}

#[test]
fn paginate_unusable_store_is_result_store_unavailable() {
    let (project, path) = project();
    std::fs::write(
        project.path().join(".planning/slugaudit/results"),
        b"not a dir",
    )
    .expect("block the results directory");
    let manager = manager(ROOMY);

    let error = fresh(&manager, &path, 5, 10).err().expect("unavailable");

    assert_eq!(code_of(&error), "result_store_unavailable");
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(
        error.data.as_ref().expect("payload")["remedy"],
        crate::error_codes::ErrorCode::ResultStoreUnavailable.remedy()
    );
}

#[test]
fn paginate_producer_failure_mid_stream_leaves_no_cursor_and_no_entry() {
    let (_project, path) = project();
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
        per_page(10),
    );

    let error = outcome.err().expect("the producer error propagates");
    assert_eq!(error.message, "producer gave up");
    let stats = manager.result_stores().current().expect("store").stats();
    assert_eq!(stats.admitted_bytes, 0);
    assert_eq!(
        stats.active.results + stats.idle.results + stats.delivered.results,
        0
    );
}

#[test]
fn paginate_raw_rows_round_trip_without_a_typed_layout() {
    let (_project, path) = project();
    let manager = manager(ROOMY);
    let raw = PageSpec {
        tool: "raw",
        columns: vec!["a".to_owned(), "b".to_owned()],
        page_bytes: 10,
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
        per_page(10),
    )
    .expect("raw page");

    assert_eq!(
        served.rows[0].cells,
        vec![Cell::Text("x".to_owned()), Cell::Null]
    );
    assert_eq!(served.page.total, Some(1));
}
