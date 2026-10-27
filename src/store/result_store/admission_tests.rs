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

#[test]
fn result_store_admission_reclaims_delivered_oldest_first_then_idle_oldest_first() {
    let unit = unit_bytes();
    let fx = fixture(unit * 3);
    let a = write_at(&fx, 10);
    let b = write_at(&fx, 20);
    let c = write_at(&fx, 30);
    fx.set_time(40);
    read_all(&fx.store, b.id);
    fx.set_time(50);
    read_all(&fx.store, a.id);

    let d = write_at(&fx, 60);

    assert!(!survives(&fx, &b), "the oldest delivered result goes first");
    assert!(survives(&fx, &a) && survives(&fx, &c) && survives(&fx, &d));

    let e = write_at(&fx, 70);

    assert!(
        !survives(&fx, &a),
        "the next delivered result goes before any idle one"
    );
    assert!(survives(&fx, &c) && survives(&fx, &d) && survives(&fx, &e));

    // Everything left goes idle; one read makes D active again.
    fx.set_time(70 + GRACE_SECS as i64 + 100);
    touch(&fx.store, d.id);
    let f = write_at(&fx, 70 + GRACE_SECS as i64 + 100);

    assert!(!survives(&fx, &c), "the oldest idle result goes first");
    assert!(survives(&fx, &d), "the re-activated result is protected");
    assert!(survives(&fx, &e) && survives(&fx, &f));
    let stats = fx.store.stats();
    assert_eq!((stats.reclaimed_delivered, stats.reclaimed_idle), (2, 1));
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
