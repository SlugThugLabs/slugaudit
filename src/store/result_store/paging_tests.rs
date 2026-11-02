use super::harness_tests::*;
use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn result_store_the_same_ordinal_read_twice_returns_identical_rows() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 50, 10).check("write")?;

    let first = fx
        .store
        .read_page(handle.id, 20, |_| Take::Yes)
        .check("first")?;
    let second = fx
        .store
        .read_page(handle.id, 20, |_| Take::Yes)
        .check("second")?;

    assert_eq!(first.rows, second.rows);
    assert_eq!(first.rows.len(), 30);
    assert_eq!(first.rows[0].ord, 20);
    Ok(())
}

/// Pages through a result `page_rows` at a time and returns every ordinal.
fn walk(store: &ResultStore, id: ResultId, page_rows: usize) -> TestResult<(Vec<u64>, usize)> {
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
            .check("page")?;
        pages += 1;
        seen.extend(page.rows.iter().map(|row| row.ord));
        match page.next_ord {
            Some(next) => from = next,
            None => return Ok((seen, pages)),
        }
    }
}

#[test]
fn result_store_pages_are_disjoint_and_cover_the_result_once_at_every_boundary() -> TestResult {
    let fx = fixture(1 << 30)?;
    for rows in [0_usize, 1, 999, 1000, 1001, 2500] {
        let handle = write_rows(&fx.store, rows, 3).check("write")?;
        assert_eq!(handle.row_count, rows as u64);
        for page_rows in [1_usize, 7, 100, 1000, 3000] {
            let (seen, pages) = walk(&fx.store, handle.id, page_rows)?;

            let expected: Vec<u64> = (0..rows as u64).collect();
            assert_eq!(seen, expected, "rows={rows} page_rows={page_rows}");
            assert_eq!(
                pages,
                rows.div_ceil(page_rows).max(1),
                "rows={rows} page_rows={page_rows}"
            );
        }
    }
    Ok(())
}

#[test]
fn result_store_an_ordinal_past_the_row_count_is_unknown() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 5, 3).check("write")?;

    let result = fx.store.read_page(handle.id, 6, |_| Take::Yes);

    assert!(
        matches!(result, Err(ResultStoreError::Unknown)),
        "{result:?}"
    );
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .unknown_cursors,
        1
    );
    Ok(())
}

#[test]
fn result_store_the_ordinal_at_the_row_count_is_an_empty_final_page() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 5, 3).check("write")?;

    let page = fx
        .store
        .read_page(handle.id, 5, |_| Take::Yes)
        .check("page")?;

    assert!(page.rows.is_empty());
    assert_eq!(page.next_ord, None);
    assert_eq!(page.row_count, 5);
    Ok(())
}

#[test]
fn result_store_an_idle_result_is_still_readable_and_active_again_after_one_read() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;
    fx.set_time(1_000 + GRACE_SECS_SIGNED + 1);
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .idle
            .results,
        1
    );

    let page = touch(&fx.store, handle.id)?;

    assert_eq!(page.rows.len(), 1);
    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!((stats.active.results, stats.idle.results), (1, 0));
    Ok(())
}

#[test]
fn result_store_the_grace_boundary_is_inclusive() -> TestResult {
    let fx = fixture(1 << 30)?;
    write_rows(&fx.store, 3, 3).check("write")?;

    fx.set_time(1_000 + GRACE_SECS_SIGNED);
    let at = fx.store.stats().check("read the store counters")?;
    fx.set_time(1_000 + GRACE_SECS_SIGNED + 1);
    let after = fx.store.stats().check("read the store counters")?;

    assert_eq!((at.active.results, after.idle.results), (1, 1));
    Ok(())
}

#[test]
fn result_store_serving_the_last_page_marks_the_result_delivered() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;
    touch(&fx.store, handle.id)?;
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .delivered
            .results,
        0
    );

    read_all(&fx.store, handle.id)?;

    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!((stats.delivered.results, stats.active.results), (1, 0));
    // A delivered result stays readable for a final-page retry.
    assert_eq!(read_all(&fx.store, handle.id)?.len(), 10);
    Ok(())
}

#[test]
fn result_store_a_result_without_complete_state_is_not_addressable() -> TestResult {
    let fx = fixture(1 << 30)?;
    let mut writer = fx.store.begin(spec(&["id"])).check("begin")?;
    writer
        .inject(super::fault::Fault::IncompleteMeta)
        .check("arm")?;
    writer.push(&[Cell::Int(1)]).check("push")?;

    let handle = writer
        .finish(&complete())
        .check("registered as incomplete")?;
    let result = fx.store.read_page(handle.id, 0, |_| Take::Yes);

    assert!(
        matches!(result, Err(ResultStoreError::Unknown)),
        "{result:?}"
    );
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_a_writer_dropped_before_finish_leaves_no_trace() -> TestResult {
    let fx = fixture(1 << 30)?;
    let mut writer = fx.store.begin(spec(&["id"])).check("begin")?;
    for index in 0..50 {
        writer.push(&[Cell::Int(index)]).check("push")?;
    }
    // The file is unlinked at creation; its descriptor still names the id.
    let open = deleted_descriptors_under(fx.dir.path())?;
    assert_eq!(open.len(), 1, "the open writer holds one descriptor");
    let stem = open[0]
        .trim_end_matches(" (deleted)")
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".db"))
        .check("descriptor named <session>-<id>.db")?;
    // The name is `<session>-<result>.db`; a UUID is 36 characters.
    let id_text = stem.get(stem.len() - 36..).check("result id suffix")?;
    let id = ResultId(uuid::Uuid::parse_str(id_text).check("file named by result id")?);

    drop(writer);

    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!(
        (
            stats.active.results + stats.idle.results + stats.delivered.results,
            stats.admitted_bytes
        ),
        (0, 0)
    );
    let outcome = read_token(&fx.store, &fx.store.cursor(id, 0).to_string());
    assert!(
        matches!(outcome, Err(ResultStoreError::Expired)),
        "{outcome:?}"
    );
    assert!(db_files(&fx.results_dir())?.is_empty());
    assert!(deleted_descriptors_under(fx.dir.path())?.is_empty());
    Ok(())
}

#[test]
fn result_store_tool_of_names_the_producer_without_renewing_or_marking() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;
    fx.set_time(1_000 + GRACE_SECS_SIGNED + 1);
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .idle
            .results,
        1,
        "idle before the call"
    );

    let tool = fx.store.tool_of(handle.id).check("tool")?;

    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!(tool, "query");
    assert_eq!((stats.idle.results, stats.active.results), (1, 0));
    assert_eq!(stats.delivered.results, 0);
    Ok(())
}

#[test]
fn result_store_tool_of_a_reclaimed_result_is_expired_and_counted_once() -> TestResult {
    let fx = fixture(1 << 30)?;
    let stale = ResultId(uuid::Uuid::new_v4());

    let error = fx.store.tool_of(stale).check_err("not registered")?;

    assert!(matches!(error, ResultStoreError::Expired));
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .expired_cursors,
        1
    );
    Ok(())
}

#[test]
fn result_store_first_row_is_offered_to_accept_but_always_taken() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 5, 3).check("write")?;
    let mut asked = Vec::new();

    let page = fx
        .store
        .read_page(handle.id, 2, |row| {
            asked.push(row.ord);
            Take::No
        })
        .check("page")?;

    assert_eq!(asked, vec![2, 3], "the first row is offered, then the next");
    assert_eq!(page.rows.len(), 1, "a No on the first row still yields it");
    assert_eq!(page.rows[0].ord, 2);
    assert_eq!(page.next_ord, Some(3));
    Ok(())
}

#[test]
fn result_store_offering_the_first_row_with_yes_everywhere_offers_every_row() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 5, 3).check("write")?;
    let mut asked = Vec::new();

    let page = fx
        .store
        .read_page(handle.id, 1, |row| {
            asked.push(row.ord);
            Take::Yes
        })
        .check("page")?;

    assert_eq!(asked, vec![1, 2, 3, 4]);
    assert_eq!(page.rows.len(), 4);
    assert_eq!(page.next_ord, None);
    Ok(())
}

#[test]
fn result_store_meta_of_returns_the_stored_meta_without_serving() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 3, 3).check("write")?;

    let meta = fx.store.meta_of(handle.id).check("meta")?;

    assert_eq!(meta.tool, "query");
    assert_eq!(meta.columns, vec!["id".to_owned(), "payload".to_owned()]);
    assert_eq!(meta.completeness, complete());
    Ok(())
}
