use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn run_blocking_propagates_a_successful_result() -> TestResult {
    let semaphore = Arc::new(Semaphore::new(4));
    let result = run_blocking(
        Arc::clone(&semaphore),
        "test",
        || Ok::<_, ErrorData>(42),
        None,
    )
    .await;
    assert_eq!(result.check("ok")?, 42);
    Ok(())
}

#[tokio::test]
async fn run_blocking_propagates_the_tools_own_typed_error() -> TestResult {
    let semaphore = Arc::new(Semaphore::new(4));
    let result: Result<(), ErrorData> = run_blocking(
        Arc::clone(&semaphore),
        "test",
        || Err(ErrorData::invalid_params("bad request", None)),
        None,
    )
    .await;
    let error = result.check_err("must propagate the tool's error, not swallow it")?;
    assert!(error.message.contains("bad request"));
    Ok(())
}

#[tokio::test]
async fn a_panic_inside_the_blocking_closure_surfaces_as_a_typed_error_not_a_crash() -> TestResult {
    let semaphore = Arc::new(Semaphore::new(4));
    let result: Result<(), ErrorData> = run_blocking(
        Arc::clone(&semaphore),
        "test",
        || std::panic::resume_unwind(Box::new("boom")),
        None,
    )
    .await;

    let error =
        result.check_err("a panicking tool must fail the call, not take down the process")?;
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.starts_with("tool task failed: "),
        "the call must carry the runner's wrapper, not a bare join error: {}",
        error.message
    );
    assert!(
        error.message.contains("panicked with message \"boom\""),
        "the join error's cause must be preserved: {}",
        error.message
    );
    Ok(())
}

/// `Semaphore::acquire_owned` fails only once the semaphore is closed. That
/// arm has its own typed error and must not run the work without a permit.
#[tokio::test]
async fn a_closed_semaphore_fails_the_call_with_its_own_typed_error_before_running_the_work()
-> TestResult {
    let semaphore = Arc::new(Semaphore::new(4));
    semaphore.close();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let work_ran = Arc::clone(&ran);

    let result: Result<(), ErrorData> = run_blocking(
        Arc::clone(&semaphore),
        "test_closed_semaphore",
        move || {
            work_ran.store(true, Ordering::SeqCst);
            Ok(())
        },
        None,
    )
    .await;

    let error = result.check_err("a closed semaphore must fail the call")?;
    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(error.message, "semaphore closed");
    assert!(
        !ran.load(Ordering::SeqCst),
        "the work must not run without a permit"
    );
    Ok(())
}

/// `Semaphore::acquire` blocks until a permit is free, so the number of
/// closures observed running concurrently can never exceed the semaphore's
/// permit count — not a probabilistic property but a guarantee `run_blocking`
/// must not violate (e.g. by leaking a permit or racing the counter update
/// outside the permit's held lifetime).
#[tokio::test]
async fn the_semaphore_bounds_concurrent_blocking_work() -> TestResult {
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
            run_blocking(
                Arc::clone(&semaphore),
                "test",
                move || {
                    let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    current.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, ErrorData>(())
                },
                None,
            )
            .await
        }));
    }
    for handle in handles {
        handle.await.check("task joins")?.check("work succeeds")?;
    }

    assert!(
        peak.load(Ordering::SeqCst) <= PERMITS,
        "observed {} concurrent blocking operations against a {}-permit semaphore",
        peak.load(Ordering::SeqCst),
        PERMITS
    );
    Ok(())
}
