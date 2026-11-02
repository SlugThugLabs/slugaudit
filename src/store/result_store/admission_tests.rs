use super::harness_tests::*;
use super::*;
use slugaudit_test_check::{Check, TestResult};

const ROWS: usize = 200;
const PAYLOAD: usize = 1000;

/// The measured size of one result of the standard shape.
fn unit_bytes() -> TestResult<u64> {
    let fx = fixture(1 << 30)?;
    Ok(write_rows(&fx.store, ROWS, PAYLOAD).check("probe")?.bytes)
}

fn write_at(fx: &Fixture, seconds: i64) -> TestResult<ResultHandle> {
    fx.set_time(seconds);
    write_rows(&fx.store, ROWS, PAYLOAD).check("write")
}

fn survives(fx: &Fixture, handle: &ResultHandle) -> TestResult<bool> {
    match fx.store.read_page(handle.id, 0, |_| Take::No) {
        Ok(_) => Ok(true),
        Err(ResultStoreError::Expired) => Ok(false),
        Err(other) => Err(format!("unexpected {other:?}")).check("read the result"),
    }
}

/// Whether `handle` is still registered, without renewing its last access.
fn registered(fx: &Fixture, handle: &ResultHandle) -> TestResult<bool> {
    match fx.store.tool_of(handle.id) {
        Ok(_) => Ok(true),
        Err(ResultStoreError::Expired) => Ok(false),
        Err(other) => Err(format!("unexpected {other:?}")).check("look up the result"),
    }
}

#[test]
fn result_store_admission_reclaims_delivered_oldest_first_before_any_idle() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit * 3)?;
    let first = write_at(&fx, 10)?;
    let second = write_at(&fx, 20)?;
    let third = write_at(&fx, 30)?;
    // Delivery order (second at 40, first at 50) is the reverse of creation order, so
    // a creation-order sort would reclaim the first result first.
    fx.set_time(40);
    read_all(&fx.store, second.id)?;
    fx.set_time(50);
    read_all(&fx.store, first.id)?;

    let fourth = write_at(&fx, 60)?;

    assert!(
        !registered(&fx, &second)?,
        "the oldest delivered result goes first"
    );
    assert!(registered(&fx, &first)? && registered(&fx, &third)? && registered(&fx, &fourth)?);

    let fifth = write_at(&fx, 70)?;

    assert!(
        !registered(&fx, &first)?,
        "the next delivered result goes before any idle one"
    );
    assert!(registered(&fx, &third)? && registered(&fx, &fourth)? && registered(&fx, &fifth)?);
    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!((stats.reclaimed_delivered, stats.reclaimed_idle), (2, 0));
    Ok(())
}

// Mutation check: sorting idle candidates by creation order (`seq`) instead
// of `last_access` reclaims A first and fails this test.
#[test]
fn result_store_admission_reclaims_idle_results_by_last_access_not_creation_order() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit * 3)?;
    let first = write_at(&fx, 10)?;
    let second = write_at(&fx, 20)?;
    let third = write_at(&fx, 30)?;
    // The first result is the oldest created but the most recently accessed: last-access
    // order is second(20), third(30), first(40).
    fx.set_time(40);
    touch(&fx.store, first.id)?;
    fx.set_time(40 + GRACE_SECS_SIGNED + 100);

    let fourth = write_at(&fx, 40 + GRACE_SECS_SIGNED + 100)?;

    assert!(
        !registered(&fx, &second)?,
        "the least recently accessed goes first"
    );
    assert!(registered(&fx, &first)? && registered(&fx, &third)? && registered(&fx, &fourth)?);

    let fifth = write_at(&fx, 40 + GRACE_SECS_SIGNED + 101)?;

    assert!(
        !registered(&fx, &third)?,
        "then the next least recently accessed"
    );
    assert!(registered(&fx, &first)? && registered(&fx, &fourth)? && registered(&fx, &fifth)?);
    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!((stats.reclaimed_delivered, stats.reclaimed_idle), (0, 2));
    Ok(())
}

#[test]
fn result_store_a_re_activated_result_is_protected_from_idle_reclaim() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit * 2)?;
    let a = write_at(&fx, 10)?;
    let b = write_at(&fx, 20)?;
    fx.set_time(20 + GRACE_SECS_SIGNED + 100);
    touch(&fx.store, a.id)?;

    let c = write_at(&fx, 20 + GRACE_SECS_SIGNED + 100)?;

    assert!(
        !registered(&fx, &b)?,
        "the idle result goes, not the active one"
    );
    assert!(registered(&fx, &a)? && registered(&fx, &c)?);
    Ok(())
}

#[test]
fn result_store_an_active_result_is_never_reclaimed_and_full_names_the_active_count() -> TestResult
{
    let unit = unit_bytes()?;
    let fx = fixture(unit * 2)?;
    let a = write_at(&fx, 10)?;
    let b = write_at(&fx, 20)?;

    fx.set_time(30);
    let refused = write_rows(&fx.store, ROWS, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::Full { active_count: 2 })),
        "{refused:?}"
    );
    assert!(survives(&fx, &a)? && survives(&fx, &b)?);
    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!((stats.refused_full, stats.active.results), (1, 2));
    Ok(())
}

#[test]
fn result_store_a_refused_result_reclaims_nothing_and_leaves_no_file() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit * 2)?;
    let active = write_at(&fx, 10)?;
    let delivered = write_at(&fx, 20)?;
    read_all(&fx.store, delivered.id)?;
    // One and a half units fits the budget alone but not beside the active
    // result, even after reclaiming the delivered one.
    fx.set_time(30);
    let refused = write_rows(&fx.store, ROWS * 3 / 2, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::Full { active_count: 1 })),
        "{refused:?}"
    );
    assert!(survives(&fx, &delivered)?, "a refusal destroys nothing");
    assert!(survives(&fx, &active)?);
    assert!(db_files(&fx.results_dir())?.is_empty());
    Ok(())
}

#[test]
fn result_store_a_result_bigger_than_the_whole_budget_is_too_large() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit / 2)?;

    let refused = write_rows(&fx.store, ROWS, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::TooLarge)),
        "{refused:?}"
    );
    let stats = fx.store.stats().check("read the store counters")?;
    assert_eq!((stats.refused_too_large, stats.admitted_bytes), (1, 0));
    Ok(())
}

#[test]
fn result_store_admitted_bytes_never_exceed_the_budget_under_churn() -> TestResult {
    let unit = unit_bytes()?;
    let budget = unit * 5 / 2;
    let fx = fixture(budget)?;

    for step in 0..12 {
        fx.set_time(100 + step * 2_000);
        let handle = write_rows(&fx.store, ROWS, PAYLOAD).check("an idle store always admits")?;
        if step % 3 == 0 {
            read_all(&fx.store, handle.id)?;
        }

        let stats = fx.store.stats().check("read the store counters")?;
        assert!(stats.admitted_bytes <= budget, "step {step}: {stats:?}");
        let held = stats.active.bytes + stats.idle.bytes + stats.delivered.bytes;
        assert_eq!(held, stats.admitted_bytes);
    }
    Ok(())
}

#[test]
fn result_store_stats_report_budget_classes_and_bytes() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit * 4)?;
    let delivered = write_at(&fx, 10)?;
    read_all(&fx.store, delivered.id)?;
    write_at(&fx, 20)?;
    fx.set_time(20 + GRACE_SECS_SIGNED + 5);
    write_rows(&fx.store, ROWS, PAYLOAD).check("write")?;

    let stats = fx.store.stats().check("read the store counters")?;

    assert_eq!(stats.budget_bytes, unit * 4);
    assert_eq!(
        (
            stats.delivered.results,
            stats.idle.results,
            stats.active.results
        ),
        (1, 1, 1)
    );
    assert_eq!(stats.admitted_bytes, unit * 3);
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_a_refused_admission_releases_the_incoming_file_and_connection() -> TestResult {
    let unit = unit_bytes()?;
    let fx = fixture(unit)?;
    let kept = write_at(&fx, 10)?;
    touch(&fx.store, kept.id)?;

    let refused = write_rows(&fx.store, ROWS, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::Full { active_count: 1 })),
        "{:?}",
        refused.err()
    );
    assert!(db_files(&fx.results_dir())?.is_empty());
    assert_eq!(deleted_descriptors_under(fx.dir.path())?.len(), 1);
    Ok(())
}
