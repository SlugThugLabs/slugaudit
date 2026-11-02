//! Tests for the shared bounded worker pool.

use super::*;
use crate::sync::discovery::FileKind;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::path::PathBuf;

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

fn pool<'a, F>(files: &'a [DiscoveredFile], deadline: &'a Deadline, per_file: F) -> Pool<'a, F> {
    Pool {
        files,
        deadline,
        pass: TEST_PASS,
        per_file,
        next_index: AtomicUsize::new(0),
        error_flag: AtomicBool::new(false),
        error_slot: Mutex::new(None),
    }
}

#[test]
fn an_empty_file_set_never_calls_the_pass() -> TestResult {
    let outcomes: Vec<()> = run(&[], &generous(), TEST_PASS, |_, _| {
        Err(PublishError::InvalidParserPackVersion("called".to_owned()))
    })
    .check("an empty run")?;
    assert!(outcomes.is_empty());
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
    assert_eq!(outcomes, expected);
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

    assert!(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "f1.rs"),
        "got {error:?}"
    );
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

    assert!(
        matches!(&error, PublishError::TimeBudgetExceeded { path, .. } if path == " while testing f0.rs"),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn the_time_budget_error_names_the_file_and_the_elapsed_milliseconds() {
    let error = time_budget_error("processing", "src/lib.rs", Duration::from_millis(1500));

    assert!(
        matches!(
            &error,
            PublishError::TimeBudgetExceeded { path, elapsed_ms: 1500 }
                if path == " while processing src/lib.rs"
        ),
        "got {error:?}"
    );
}

#[test]
fn an_outcome_that_cannot_be_delivered_fails_the_run_with_the_file_named() -> TestResult {
    let files = files(1);
    let deadline = generous();
    let pool = pool(&files, &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });
    let (tx, rx) = mpsc::channel::<(usize, ())>();
    drop(rx);

    pool.run_worker(&tx);

    let recorded = lock(&pool.error_slot, "test")
        .check("lock the error slot")?
        .take();
    assert!(
        matches!(&recorded, Some(PublishError::SamplingResultLost { path, .. }) if path == "f0.rs"),
        "got {recorded:?}"
    );
    Ok(())
}

#[test]
fn a_channel_outcome_with_an_out_of_range_index_fails_the_run() -> TestResult {
    let files = files(1);
    let deadline = generous();
    let pool = pool(&files, &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });
    let (tx, rx) = mpsc::channel::<(usize, ())>();
    tx.send((1, ())).check("send the out-of-range outcome")?;
    drop(tx);

    let error = pool
        .collect(&rx)
        .check_err("an out-of-range index must fail the run")?;

    assert!(
        matches!(
            error,
            PublishError::SamplingIndexOutOfRange { index: 1, total: 1 }
        ),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn a_file_with_no_delivered_outcome_fails_the_run() -> TestResult {
    let files = files(2);
    let deadline = generous();
    let pool = pool(&files, &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });
    let (tx, rx) = mpsc::channel::<(usize, ())>();
    tx.send((0, ())).check("send one outcome")?;
    drop(tx);

    let error = pool
        .collect(&rx)
        .check_err("a missing outcome must fail the run")?;

    assert!(
        matches!(&error, PublishError::SamplingResultLost { path, .. } if path == "f1.rs"),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn a_later_error_is_logged_once_under_the_pass_and_the_first_is_returned() -> TestResult {
    const SUPERSEDED_PATH: &str = "superseded-second-error-probe.rs";
    let deadline = generous();
    let pool = pool(&[], &deadline, |_: usize, _: &DiscoveredFile| {
        Ok::<(), PublishError>(())
    });

    let ((), logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-superseded-error-probe");
        pool.abort(PublishError::ImportTooLarge {
            total: 10,
            limit: 9,
        });
        pool.abort(PublishError::ChangedDuringSample {
            path: SUPERSEDED_PATH.to_owned(),
        });
    })?;

    let (tx, rx) = mpsc::channel::<(usize, ())>();
    drop(tx);
    let returned = pool.collect(&rx).check_err("the run fails")?;
    assert!(
        matches!(returned, PublishError::ImportTooLarge { .. }),
        "the first error must stay the returned one: {returned:?}"
    );
    assert!(
        logs.contains("control-line-superseded-error-probe"),
        "the capture must be live: {logs}"
    );
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
        assert!(line.contains(expected), "missing {expected}: {line}");
    }
    Ok(())
}
