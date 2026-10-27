use super::harness_tests::*;
use super::*;

const ROWS: usize = 200;
const PAYLOAD: usize = 1000;

/// The measured size of one result of the standard shape.
fn unit_bytes() -> u64 {
    let fx = fixture(1 << 30);
    write_rows(&fx.store, ROWS, PAYLOAD).expect("probe").bytes
}

fn write_at(fx: &Fixture, seconds: i64) -> ResultHandle {
    fx.set_time(seconds);
    write_rows(&fx.store, ROWS, PAYLOAD).expect("write")
}

fn survives(fx: &Fixture, handle: &ResultHandle) -> bool {
    match fx.store.read_page(handle.id, 0, |_| Take::No) {
        Ok(_) => true,
        Err(ResultStoreError::Expired) => false,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

/// Whether `handle` is still registered, without renewing its last access.
fn registered(fx: &Fixture, handle: &ResultHandle) -> bool {
    match fx.store.tool_of(handle.id) {
        Ok(_) => true,
        Err(ResultStoreError::Expired) => false,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

#[test]
fn result_store_admission_reclaims_delivered_oldest_first_before_any_idle() {
    let unit = unit_bytes();
    let fx = fixture(unit * 3);
    let a = write_at(&fx, 10);
    let b = write_at(&fx, 20);
    let c = write_at(&fx, 30);
    // Delivery order (B at 40, A at 50) is the reverse of creation order, so
    // a creation-order sort would reclaim A first.
    fx.set_time(40);
    read_all(&fx.store, b.id);
    fx.set_time(50);
    read_all(&fx.store, a.id);

    let d = write_at(&fx, 60);

    assert!(
        !registered(&fx, &b),
        "the oldest delivered result goes first"
    );
    assert!(registered(&fx, &a) && registered(&fx, &c) && registered(&fx, &d));

    let e = write_at(&fx, 70);

    assert!(
        !registered(&fx, &a),
        "the next delivered result goes before any idle one"
    );
    assert!(registered(&fx, &c) && registered(&fx, &d) && registered(&fx, &e));
    let stats = fx.store.stats();
    assert_eq!((stats.reclaimed_delivered, stats.reclaimed_idle), (2, 0));
}

// Mutation check: sorting idle candidates by creation order (`seq`) instead
// of `last_access` reclaims A first and fails this test.
#[test]
fn result_store_admission_reclaims_idle_results_by_last_access_not_creation_order() {
    let unit = unit_bytes();
    let fx = fixture(unit * 3);
    let a = write_at(&fx, 10);
    let b = write_at(&fx, 20);
    let c = write_at(&fx, 30);
    // A is the oldest created but the most recently accessed: last-access
    // order is B(20), C(30), A(40).
    fx.set_time(40);
    touch(&fx.store, a.id);
    fx.set_time(40 + GRACE_SECS as i64 + 100);

    let d = write_at(&fx, 40 + GRACE_SECS as i64 + 100);

    assert!(
        !registered(&fx, &b),
        "the least recently accessed goes first"
    );
    assert!(registered(&fx, &a) && registered(&fx, &c) && registered(&fx, &d));

    let e = write_at(&fx, 40 + GRACE_SECS as i64 + 101);

    assert!(
        !registered(&fx, &c),
        "then the next least recently accessed"
    );
    assert!(registered(&fx, &a) && registered(&fx, &d) && registered(&fx, &e));
    let stats = fx.store.stats();
    assert_eq!((stats.reclaimed_delivered, stats.reclaimed_idle), (0, 2));
}

#[test]
fn result_store_a_re_activated_result_is_protected_from_idle_reclaim() {
    let unit = unit_bytes();
    let fx = fixture(unit * 2);
    let a = write_at(&fx, 10);
    let b = write_at(&fx, 20);
    fx.set_time(20 + GRACE_SECS as i64 + 100);
    touch(&fx.store, a.id);

    let c = write_at(&fx, 20 + GRACE_SECS as i64 + 100);

    assert!(
        !registered(&fx, &b),
        "the idle result goes, not the active one"
    );
    assert!(registered(&fx, &a) && registered(&fx, &c));
}

#[test]
fn result_store_an_active_result_is_never_reclaimed_and_full_names_the_active_count() {
    let unit = unit_bytes();
    let fx = fixture(unit * 2);
    let a = write_at(&fx, 10);
    let b = write_at(&fx, 20);

    fx.set_time(30);
    let refused = write_rows(&fx.store, ROWS, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::Full { active_count: 2 })),
        "{refused:?}"
    );
    assert!(survives(&fx, &a) && survives(&fx, &b));
    let stats = fx.store.stats();
    assert_eq!((stats.refused_full, stats.active.results), (1, 2));
}

#[test]
fn result_store_a_refused_result_reclaims_nothing_and_leaves_no_file() {
    let unit = unit_bytes();
    let fx = fixture(unit * 2);
    let active = write_at(&fx, 10);
    let delivered = write_at(&fx, 20);
    read_all(&fx.store, delivered.id);
    // One and a half units fits the budget alone but not beside the active
    // result, even after reclaiming the delivered one.
    fx.set_time(30);
    let refused = write_rows(&fx.store, ROWS * 3 / 2, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::Full { active_count: 1 })),
        "{refused:?}"
    );
    assert!(survives(&fx, &delivered), "a refusal destroys nothing");
    assert!(survives(&fx, &active));
    assert!(db_files(&fx.results_dir()).is_empty());
}

#[test]
fn result_store_a_result_bigger_than_the_whole_budget_is_too_large() {
    let unit = unit_bytes();
    let fx = fixture(unit / 2);

    let refused = write_rows(&fx.store, ROWS, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::TooLarge)),
        "{refused:?}"
    );
    let stats = fx.store.stats();
    assert_eq!((stats.refused_too_large, stats.admitted_bytes), (1, 0));
}

#[test]
fn result_store_admitted_bytes_never_exceed_the_budget_under_churn() {
    let unit = unit_bytes();
    let budget = unit * 5 / 2;
    let fx = fixture(budget);

    for step in 0..12 {
        fx.set_time(100 + step * 2_000);
        let handle = write_rows(&fx.store, ROWS, PAYLOAD).expect("an idle store always admits");
        if step % 3 == 0 {
            read_all(&fx.store, handle.id);
        }

        let stats = fx.store.stats();
        assert!(stats.admitted_bytes <= budget, "step {step}: {stats:?}");
        let held = stats.active.bytes + stats.idle.bytes + stats.delivered.bytes;
        assert_eq!(held, stats.admitted_bytes);
    }
}

#[test]
fn result_store_stats_report_budget_classes_and_bytes() {
    let unit = unit_bytes();
    let fx = fixture(unit * 4);
    let delivered = write_at(&fx, 10);
    read_all(&fx.store, delivered.id);
    write_at(&fx, 20);
    fx.set_time(20 + GRACE_SECS as i64 + 5);
    write_rows(&fx.store, ROWS, PAYLOAD).expect("write");

    let stats = fx.store.stats();

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
}

#[test]
#[cfg(target_os = "linux")]
fn result_store_a_refused_admission_releases_the_incoming_file_and_connection() {
    let unit = unit_bytes();
    let fx = fixture(unit);
    let kept = write_at(&fx, 10);
    touch(&fx.store, kept.id);

    let refused = write_rows(&fx.store, ROWS, PAYLOAD);

    assert!(
        matches!(refused, Err(ResultStoreError::Full { active_count: 1 })),
        "{:?}",
        refused.err()
    );
    assert!(db_files(&fx.results_dir()).is_empty());
    assert_eq!(deleted_descriptors_under(fx.dir.path()).len(), 1);
}
