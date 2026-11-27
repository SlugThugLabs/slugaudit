//! Tests for the shared bounded worker pool and persistent multi-chunk runner.

use super::*;
use crate::sync::discovery::FileKind;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

const TEST_PASS: Pass = Pass {
    operation: "test_pass",
    activity: "testing",
};

fn files(count: usize) -> Vec<DiscoveredFile> {
    (0..count)
        .map(|idx| DiscoveredFile {
            relative_path: format!("f{idx}.rs"),
            absolute_path: PathBuf::from(format!("/nonexistent/f{idx}.rs")),
            kind: FileKind::Indexed,
        })
        .collect()
}

fn generous() -> Deadline {
    Deadline::after(Duration::from_mins(1))
}

fn shared_pool<'a, F>(
    files: &'a [DiscoveredFile],
    deadline: &'a Deadline,
    per_file: F,
) -> (SharedPool<'a, DiscoveredFile, F>, Batch) {
    (
        SharedPool {
            files,
            deadline,
            pass: TEST_PASS,
            per_file,
        },
        Batch::new(0, files.len(), TEST_PASS),
    )
}

#[test]
fn an_empty_file_set_never_calls_the_pass() -> TestResult {
    let outcomes: Vec<()> = run::<DiscoveredFile, _, _>(&[], &generous(), TEST_PASS, |_, _| {
        Err(PublishError::InvalidParserPackVersion("called".to_owned()))
    })
    .check("an empty run")?;
    ensure(
        outcomes.is_empty(),
        "empty input produces empty outcome list",
    )?;
    Ok(())
}

#[test]
fn outcomes_come_back_in_file_order_whichever_worker_finished_first() -> TestResult {
    let files = files(200);

    let outcomes = run(&files, &generous(), TEST_PASS, |idx, file| {
        Ok((idx, file.relative_path.clone()))
    })
    .check("run the pool")?;

    let expected: Vec<(usize, String)> = (0..200).map(|idx| (idx, format!("f{idx}.rs"))).collect();
    ensure_eq(
        &outcomes,
        &expected,
        "pool outcomes preserve input slice order",
    )?;
    Ok(())
}

#[test]
fn with_pool_reuses_persistent_worker_threads_across_chunks() -> TestResult {
    let files = files(32);
    let mut all = Vec::new();
    let mut thread_ids = HashSet::new();

    with_pool(
        &files,
        &generous(),
        TEST_PASS,
        |idx, file| Ok((idx, file.relative_path.clone(), thread::current().id())),
        |runner| {
            while let Some(chunk) = runner.next_chunk(4)? {
                for (idx, path, tid) in chunk {
                    thread_ids.insert(tid);
                    all.push((idx, path));
                }
            }
            Ok(())
        },
    )
    .check("multi-chunk with_pool")?;

    let expected: Vec<(usize, String)> = (0..32).map(|idx| (idx, format!("f{idx}.rs"))).collect();
    ensure_eq(&all, &expected, "all 8 chunks preserve global file order")?;
    ensure(
        thread_ids.len() <= MAX_CONCURRENT_BLOCKING_OPS,
        format!(
            "persistent pool spawned {} distinct threads across 8 chunks (cap {})",
            thread_ids.len(),
            MAX_CONCURRENT_BLOCKING_OPS
        ),
    )?;
    Ok(())
}

#[test]
fn a_per_file_error_fails_the_run_with_that_error() -> TestResult {
    let files = files(3);

    let error = run(&files, &generous(), TEST_PASS, |idx, file| {
        if idx == 1 {
            Err(PublishError::ChangedDuringSample {
                path: file.relative_path.clone(),
            })
        } else {
            Ok(())
        }
    })
    .check_err("the failing file must fail the run")?;

    ensure(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "f1.rs"),
        format!("got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_spent_deadline_names_the_pass_activity_and_the_file() -> TestResult {
    let files = files(1);

    let error = run(
        &files,
        &Deadline::after(Duration::ZERO),
        TEST_PASS,
        |_, _| Ok(()),
    )
    .check_err("the deadline must trip")?;

    ensure(
        matches!(
            &error,
            PublishError::TimeBudgetExceeded {
                activity: "testing",
                path: Some(p),
                ..
            } if p == "f0.rs"
        ),
        format!("got {error:?}"),
    )?;
    ensure(
        error.to_string().contains("while testing f0.rs"),
        format!("display message names activity and file: {error}"),
    )?;
    Ok(())
}

#[test]
fn the_time_budget_error_names_the_file_and_the_elapsed_milliseconds() -> TestResult {
    let error =
        PublishError::time_budget_exceeded("processing", "src/lib.rs", Duration::from_millis(1500));

    ensure(
        matches!(
            &error,
            PublishError::TimeBudgetExceeded {
                activity: "processing",
                path: Some(p),
                elapsed_ms: 1500,
            } if p == "src/lib.rs"
        ),
        format!("got {error:?}"),
    )?;
    ensure(
        error.to_string().contains("while processing src/lib.rs"),
        format!("display message names processing and src/lib.rs: {error}"),
    )?;
    Ok(())
}

#[test]
fn an_outcome_that_cannot_be_delivered_fails_the_run_with_the_file_named() -> TestResult {
    let files = files(1);
    let deadline = generous();
    let (pool, batch) = shared_pool(&files, &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });
    let (tx, rx) = mpsc::channel::<(usize, ())>();
    drop(rx);

    pool.run_worker(&batch, &tx);

    let recorded = lock(&batch.error_slot, "test")
        .check("lock the error slot")?
        .take();
    ensure(
        matches!(&recorded, Some(PublishError::SamplingResultLost { path, .. }) if path == "f0.rs"),
        format!("got {recorded:?}"),
    )?;
    Ok(())
}

#[test]
fn a_channel_outcome_with_an_out_of_range_index_fails_the_run() -> TestResult {
    let files = files(1);
    let deadline = generous();
    let (_pool, batch) = shared_pool(&files, &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });
    let (tx, rx) = mpsc::channel::<(usize, ())>();
    tx.send((1, ())).check("send the out-of-range outcome")?;
    drop(tx);

    let error = batch
        .collect(&files, &rx)
        .check_err("an out-of-range index must fail the run")?;

    ensure(
        matches!(
            error,
            PublishError::SamplingIndexOutOfRange { index: 1, total: 1 }
        ),
        format!("got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_file_with_no_delivered_outcome_fails_the_run() -> TestResult {
    let files = files(2);
    let deadline = generous();
    let (_pool, batch) = shared_pool(&files, &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });
    let (tx, rx) = mpsc::channel::<(usize, ())>();
    tx.send((0, ())).check("send one outcome")?;
    drop(tx);

    let error = batch
        .collect(&files, &rx)
        .check_err("a missing outcome must fail the run")?;

    ensure(
        matches!(&error, PublishError::SamplingResultLost { path, .. } if path == "f1.rs"),
        format!("got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_later_error_is_logged_once_under_the_pass_and_the_first_is_returned() -> TestResult {
    const SUPERSEDED_PATH: &str = "superseded-second-error-probe.rs";
    let deadline = generous();
    let (files, (_pool, batch)) = {
        let empty = Vec::new();
        let (_, b) = shared_pool(&empty, &deadline, |_: usize, _: &DiscoveredFile| {
            Ok::<(), PublishError>(())
        });
        (empty, ((), b))
    };

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-superseded-error-probe");
        batch.abort(PublishError::ImportTooLarge {
            total: 10,
            limit: 9,
        });
        batch.abort(PublishError::ChangedDuringSample {
            path: SUPERSEDED_PATH.to_owned(),
        });
    })?;

    let (tx, rx) = mpsc::channel::<(usize, ())>();
    drop(tx);
    let returned = batch.collect(&files, &rx).check_err("the run fails")?;
    ensure(
        matches!(returned, PublishError::ImportTooLarge { .. }),
        format!("the first error must stay the returned one: {returned:?}"),
    )?;
    ensure(
        logs.contains("control-line-superseded-error-probe"),
        format!("the capture must be live: {logs}"),
    )?;
    let logged: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains(SUPERSEDED_PATH))
        .collect();
    let [line] = logged.as_slice() else {
        return Err(format!(
            "the later error must be logged exactly once: {logs}"
        ))
        .check("superseded log lines");
    };
    for expected in [
        "operation=\"test_pass\"",
        "error_kind=\"changed_during_sample\"",
        &format!("path=Some(\"{SUPERSEDED_PATH}\")"),
        "changed on disk after being sampled",
        "a later worker failure was discarded",
    ] {
        ensure(
            line.contains(expected),
            format!("missing {expected}: {line}"),
        )?;
    }
    Ok(())
}
