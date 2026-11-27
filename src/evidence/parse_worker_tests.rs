use super::{WorkerPool, process_bounded, run_job_on_pool};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tree_sitter_language_pack::{Error as PackError, ProcessConfig};

#[test]
fn worker_pool_reuses_warm_thread_across_caller_threads_and_retires_on_timeout() -> TestResult {
    let pool = Arc::new(WorkerPool::new());
    let ok_cfg = ProcessConfig::new("rust")
        .all()
        .with_parse_timeout_ms(Some(60_000));

    let first = run_job_on_pool(&pool, "pub fn alpha() {}\n", ok_cfg.clone(), 60_000, None)
        .check("first parse")?;
    ensure_eq(&first.structure.len(), &1usize, "alpha structure extracted")?;
    let ids_after_first = pool.idle_thread_ids();
    let first_tid = *ids_after_first
        .first()
        .check("one warm idle worker after first parse")?;

    let pool_clone = Arc::clone(&pool);
    let second_cfg = ok_cfg.clone();
    let handle = thread::spawn(move || {
        run_job_on_pool(&pool_clone, "pub fn beta() {}\n", second_cfg, 60_000, None)
    });
    let second = crate::util::join_thread(handle)
        .check("second caller thread joined")?
        .check("second parse")?;
    ensure_eq(&second.structure.len(), &1usize, "beta structure extracted")?;
    let ids_after_second = pool.idle_thread_ids();
    ensure_eq(
        &ids_after_second,
        &vec![first_tid],
        "process-wide pool reuses the same warm worker thread across different caller threads",
    )?;

    let bad_cfg = ProcessConfig::new("no-such-language-for-worker-reuse")
        .all()
        .with_parse_timeout_ms(Some(60_000));
    let _non_timeout_err =
        run_job_on_pool(&pool, "x", bad_cfg, 60_000, None).check_err("unknown language fails")?;
    ensure_eq(
        &pool.idle_thread_ids(),
        &vec![first_tid],
        "non-timeout error keeps the pooled worker alive",
    )?;

    let big = "pub fn slow(v: Option<u8>) -> Option<u8> { v }\n".repeat(10_000);
    let timeout_cfg = ProcessConfig::new("rust")
        .all()
        .with_parse_timeout_ms(Some(1));
    let timeout_err =
        run_job_on_pool(&pool, &big, timeout_cfg, 1, None).check_err("1ms parse of big source")?;
    ensure(
        matches!(timeout_err, PackError::ParseTimeout { .. }),
        format!("expected ParseTimeout, got {timeout_err:?}"),
    )?;
    ensure(
        pool.idle_thread_ids().is_empty(),
        "timeout retires the worker so its un-reset parser cache is discarded",
    )?;

    let third = run_job_on_pool(&pool, "pub fn gamma() {}\n", ok_cfg.clone(), 60_000, None)
        .check("parse after timeout")?;
    ensure_eq(&third.structure.len(), &1usize, "gamma structure extracted")?;
    let third_tid = *pool
        .idle_thread_ids()
        .first()
        .check("fresh worker spawned after timeout")?;
    ensure_ne(
        &first_tid,
        &third_tid,
        "parse after timeout lazily spawns a fresh worker thread",
    )?;

    let global = process_bounded("pub fn smoke() {}\n", ok_cfg, 60_000).check("global pool")?;
    ensure_eq(&global.structure.len(), &1usize, "global pool smoke parse")?;
    Ok(())
}

#[test]
fn wedged_worker_hits_recv_timeout_and_releases_pool_slot_without_hanging() -> TestResult {
    let pool = WorkerPool::with_grace(Duration::from_millis(10));
    let cfg = ProcessConfig::new("rust")
        .all()
        .with_parse_timeout_ms(Some(60_000));

    let started = std::time::Instant::now();
    let err = run_job_on_pool(
        &pool,
        "pub fn delayed() {}\n",
        cfg.clone(),
        5,
        Some(Duration::from_millis(250)),
    )
    .check_err("simulated wedged worker must time out on reply_rx")?;

    ensure(
        matches!(err, PackError::ParseTimeout { timeout_ms: 5 }),
        format!("expected ParseTimeout(5), got {err:?}"),
    )?;
    ensure(
        started.elapsed() < Duration::from_millis(200),
        format!(
            "caller returned via recv_timeout without joining wedged thread: {:?}",
            started.elapsed()
        ),
    )?;
    ensure_eq(
        &pool.checked_out_count(),
        &0usize,
        "SlotLease releases checked_out count after discarding wedged worker",
    )?;
    ensure(
        pool.idle_thread_ids().is_empty(),
        "wedged worker is discarded, never returned to idle pool",
    )?;

    let recovered =
        run_job_on_pool(&pool, "pub fn fresh() {}\n", cfg, 60_000, None).check("fresh recovery")?;
    ensure_eq(
        &recovered.structure.len(),
        &1usize,
        "subsequent parse succeeds on a fresh worker",
    )?;
    Ok(())
}
