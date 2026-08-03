use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn run_blocking_propagates_a_successful_result() {
    let semaphore = Semaphore::new(4);
    let result = run_blocking(&semaphore, || Ok::<_, ErrorData>(42)).await;
    assert_eq!(result.expect("ok"), 42);
}

#[tokio::test]
async fn run_blocking_propagates_the_tools_own_typed_error() {
    let semaphore = Semaphore::new(4);
    let result: Result<(), ErrorData> = run_blocking(&semaphore, || {
        Err(ErrorData::invalid_params("bad request", None))
    })
    .await;
    let error = result.expect_err("must propagate the tool's error, not swallow it");
    assert!(error.message.contains("bad request"));
}

#[tokio::test]
async fn a_panic_inside_the_blocking_closure_surfaces_as_a_typed_error_not_a_crash() {
    let semaphore = Semaphore::new(4);
    let result: Result<(), ErrorData> = run_blocking(&semaphore, || panic!("boom")).await;
    assert!(
        result.is_err(),
        "a panicking tool must fail the call, not take down the process"
    );
}

/// `Semaphore::acquire` blocks until a permit is free, so the number of
/// closures observed running concurrently can never exceed the semaphore's
/// permit count — not a probabilistic property but a guarantee `run_blocking`
/// must not violate (e.g. by leaking a permit or racing the counter update
/// outside the permit's held lifetime).
#[tokio::test]
async fn the_semaphore_bounds_concurrent_blocking_work() {
    const PERMITS: usize = 2;
    const TASKS: usize = 6;
    let semaphore = Arc::new(Semaphore::new(PERMITS));
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..TASKS {
        let semaphore = Arc::clone(&semaphore);
        let current = Arc::clone(&current);
        let peak = Arc::clone(&peak);
        handles.push(tokio::spawn(async move {
            run_blocking(&semaphore, move || {
                let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                current.fetch_sub(1, Ordering::SeqCst);
                Ok::<_, ErrorData>(())
            })
            .await
        }));
    }
    for handle in handles {
        handle.await.expect("task joins").expect("work succeeds");
    }

    assert!(
        peak.load(Ordering::SeqCst) <= PERMITS,
        "observed {} concurrent blocking operations against a {}-permit semaphore",
        peak.load(Ordering::SeqCst),
        PERMITS
    );
}
