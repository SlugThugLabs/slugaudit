use super::harness_tests::*;
use super::*;

#[test]
fn result_store_the_same_ordinal_read_twice_returns_identical_rows() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 50, 10).expect("write");

    let first = fx
        .store
        .read_page(handle.id, 20, |_| Take::Yes)
        .expect("first");
    let second = fx
        .store
        .read_page(handle.id, 20, |_| Take::Yes)
        .expect("second");

    assert_eq!(first.rows, second.rows);
    assert_eq!(first.rows.len(), 30);
    assert_eq!(first.rows[0].ord, 20);
}

/// Pages through a result `page_rows` at a time and returns every ordinal.
fn walk(store: &ResultStore, id: ResultId, page_rows: usize) -> (Vec<u64>, usize) {
    let (mut seen, mut pages, mut from) = (Vec::new(), 0, 0);
    loop {
        let mut taken = 0;
        let page = store
            .read_page(id, from, |_| {
                taken += 1;
                if taken <= page_rows {
                    Take::Yes
                } else {
                    Take::No
                }
            })
            .expect("page");
        pages += 1;
        seen.extend(page.rows.iter().map(|row| row.ord));
        match page.next_ord {
            Some(next) => from = next,
            None => return (seen, pages),
        }
    }
}

#[test]
fn result_store_pages_are_disjoint_and_cover_the_result_once_at_every_boundary() {
    let fx = fixture(1 << 30);
    for rows in [0_usize, 1, 999, 1000, 1001, 2500] {
        let handle = write_rows(&fx.store, rows, 3).expect("write");
        assert_eq!(handle.row_count, rows as u64);
        for page_rows in [1_usize, 7, 100, 1000, 3000] {
            let (seen, pages) = walk(&fx.store, handle.id, page_rows);

            let expected: Vec<u64> = (0..rows as u64).collect();
            assert_eq!(seen, expected, "rows={rows} page_rows={page_rows}");
            assert_eq!(
                pages,
                rows.div_ceil(page_rows).max(1),
                "rows={rows} page_rows={page_rows}"
            );
        }
    }
}

#[test]
fn result_store_an_ordinal_past_the_row_count_is_unknown() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 5, 3).expect("write");

    let result = fx.store.read_page(handle.id, 6, |_| Take::Yes);

    assert!(
        matches!(result, Err(ResultStoreError::Unknown)),
        "{result:?}"
    );
    assert_eq!(fx.store.stats().unknown_cursors, 1);
}

#[test]
fn result_store_the_ordinal_at_the_row_count_is_an_empty_final_page() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 5, 3).expect("write");

    let page = fx
        .store
        .read_page(handle.id, 5, |_| Take::Yes)
        .expect("page");

    assert!(page.rows.is_empty());
    assert_eq!(page.next_ord, None);
    assert_eq!(page.row_count, 5);
}

#[test]
fn result_store_an_idle_result_is_still_readable_and_active_again_after_one_read() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    fx.set_time(1_000 + GRACE_SECS as i64 + 1);
    assert_eq!(fx.store.stats().idle.results, 1);

    let page = touch(&fx.store, handle.id);

    assert_eq!(page.rows.len(), 1);
    let stats = fx.store.stats();
    assert_eq!((stats.active.results, stats.idle.results), (1, 0));
}

#[test]
fn result_store_the_grace_boundary_is_inclusive() {
    let fx = fixture(1 << 30);
    write_rows(&fx.store, 3, 3).expect("write");

    fx.set_time(1_000 + GRACE_SECS as i64);
    let at = fx.store.stats();
    fx.set_time(1_000 + GRACE_SECS as i64 + 1);
    let after = fx.store.stats();

    assert_eq!((at.active.results, after.idle.results), (1, 1));
}

#[test]
fn result_store_serving_the_last_page_marks_the_result_delivered() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    touch(&fx.store, handle.id);
    assert_eq!(fx.store.stats().delivered.results, 0);

    read_all(&fx.store, handle.id);

    let stats = fx.store.stats();
    assert_eq!((stats.delivered.results, stats.active.results), (1, 0));
    // A delivered result stays readable for a final-page retry.
    assert_eq!(read_all(&fx.store, handle.id).len(), 10);
}

#[test]
fn result_store_a_result_without_complete_state_is_not_addressable() {
    let fx = fixture(1 << 30);
    let mut writer = fx.store.begin(spec(&["id"])).expect("begin");
    writer
        .inject(super::fault::Fault::IncompleteMeta)
        .expect("arm");
    writer.push(&[Cell::Int(1)]).expect("push");

    let handle = writer
        .finish(&complete())
        .expect("registered as incomplete");
    let result = fx.store.read_page(handle.id, 0, |_| Take::Yes);

    assert!(
        matches!(result, Err(ResultStoreError::Unknown)),
        "{result:?}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_a_writer_dropped_before_finish_leaves_no_trace() {
    let fx = fixture(1 << 30);
    let mut writer = fx.store.begin(spec(&["id"])).expect("begin");
    for index in 0..50 {
        writer.push(&[Cell::Int(index)]).expect("push");
    }
    // The file is unlinked at creation; its descriptor still names the id.
    let open = deleted_descriptors_under(fx.dir.path());
    assert_eq!(open.len(), 1, "the open writer holds one descriptor");
    let stem = open[0]
        .trim_end_matches(" (deleted)")
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".db"))
        .expect("descriptor named <session>-<id>.db");
    // The name is `<session>-<result>.db`; a UUID is 36 characters.
    let id_text = stem.get(stem.len() - 36..).expect("result id suffix");
    let id = ResultId(uuid::Uuid::parse_str(id_text).expect("file named by result id"));

    drop(writer);

    let stats = fx.store.stats();
    assert_eq!(
        (
            stats.active.results + stats.idle.results + stats.delivered.results,
            stats.admitted_bytes
        ),
        (0, 0)
    );
    let outcome = fx
        .store
        .read_cursor(&fx.store.cursor(id, 0).to_string(), |_| Take::Yes);
    assert!(
        matches!(outcome, Err(ResultStoreError::Expired)),
        "{outcome:?}"
    );
    assert!(db_files(&fx.results_dir()).is_empty());
    assert!(deleted_descriptors_under(fx.dir.path()).is_empty());
}

/// Rows of the standard shape whose `from_values` fails for id 9.
#[derive(Debug)]
struct FailsAtNine;

impl Materialize for FailsAtNine {
    fn columns() -> &'static [&'static str] {
        &["id", "payload"]
    }

    fn to_values(&self) -> Vec<Cell> {
        vec![Cell::Int(0), Cell::Text(String::new())]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        match values {
            [Cell::Int(9), _] => Err(ResultStoreError::Invalid {
                reason: "nine is not convertible",
            }),
            _ => Ok(Self),
        }
    }
}

#[test]
fn result_store_a_failed_typed_conversion_on_the_last_page_leaves_the_result_untouched() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    fx.set_time(1_000 + GRACE_SECS as i64 + 100);

    let first = fx
        .store
        .read_typed::<FailsAtNine>(handle.id, 7, |_| Take::Yes);
    let retry = fx
        .store
        .read_typed::<FailsAtNine>(handle.id, 7, |_| Take::Yes);

    assert!(
        matches!(first, Err(ResultStoreError::Invalid { .. })),
        "{first:?}"
    );
    assert!(
        matches!(retry, Err(ResultStoreError::Invalid { .. })),
        "{retry:?}"
    );
    let stats = fx.store.stats();
    assert_eq!((stats.delivered.results, stats.idle.results), (0, 1));
}

#[test]
fn result_store_a_failed_conversion_of_the_first_row_leaves_the_result_untouched() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    fx.set_time(1_000 + GRACE_SECS as i64 + 100);

    let outcome = fx
        .store
        .read_typed::<FailsAtNine>(handle.id, 9, |_| Take::Yes);

    assert!(
        matches!(outcome, Err(ResultStoreError::Invalid { .. })),
        "{outcome:?}"
    );
    let stats = fx.store.stats();
    assert_eq!((stats.delivered.results, stats.idle.results), (0, 1));
}

#[test]
fn result_store_tool_of_names_the_producer_without_renewing_or_marking() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    fx.set_time(1_000 + i64::try_from(GRACE_SECS).expect("grace fits") + 1);
    assert_eq!(fx.store.stats().idle.results, 1, "idle before the call");

    let tool = fx.store.tool_of(handle.id).expect("tool");

    let stats = fx.store.stats();
    assert_eq!(tool, "query");
    assert_eq!((stats.idle.results, stats.active.results), (1, 0));
    assert_eq!(stats.delivered.results, 0);
}

#[test]
fn result_store_tool_of_a_reclaimed_result_is_expired_and_counted_once() {
    let fx = fixture(1 << 30);
    let stale = ResultId(uuid::Uuid::new_v4());

    let error = fx.store.tool_of(stale).expect_err("not registered");

    assert!(matches!(error, ResultStoreError::Expired));
    assert_eq!(fx.store.stats().expired_cursors, 1);
}

#[test]
fn result_store_first_row_is_offered_to_accept_but_always_taken() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 5, 3).expect("write");
    let mut asked = Vec::new();

    let page = fx
        .store
        .read_page(handle.id, 2, |row| {
            asked.push(row.ord);
            Take::No
        })
        .expect("page");

    assert_eq!(asked, vec![2, 3], "the first row is offered, then the next");
    assert_eq!(page.rows.len(), 1, "a No on the first row still yields it");
    assert_eq!(page.rows[0].ord, 2);
    assert_eq!(page.next_ord, Some(3));
}

#[test]
fn result_store_offering_the_first_row_with_yes_everywhere_offers_every_row() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 5, 3).expect("write");
    let mut asked = Vec::new();

    let page = fx
        .store
        .read_page(handle.id, 1, |row| {
            asked.push(row.ord);
            Take::Yes
        })
        .expect("page");

    assert_eq!(asked, vec![1, 2, 3, 4]);
    assert_eq!(page.rows.len(), 4);
    assert_eq!(page.next_ord, None);
}

#[test]
fn result_store_meta_of_returns_the_stored_meta_without_serving() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 3, 3).expect("write");

    let meta = fx.store.meta_of(handle.id).expect("meta");

    assert_eq!(meta.tool, "query");
    assert_eq!(meta.columns, vec!["id".to_owned(), "payload".to_owned()]);
    assert_eq!(meta.completeness, complete());
}
