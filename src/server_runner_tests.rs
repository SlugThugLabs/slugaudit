use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
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
    ensure_eq(&result.check("ok")?, &42, "result.check(\"ok\")? == 42")?;
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
    ensure(
        error.message.contains("bad request"),
        "error.message.contains(\"bad request\")",
    )?;
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
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure(
        error.message.starts_with("tool task failed: "),
        format!(
            "the call must carry the runner's wrapper, not a bare join error: {}",
            error.message
        ),
    )?;
    ensure(
        error.message.contains("panicked with message \"boom\""),
        format!(
            "the join error's cause must be preserved: {}",
            error.message
        ),
    )?;
    Ok(())
}

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
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INTERNAL_ERROR,
        "error.code == rmcp::model::ErrorCode::INTERNAL_ERROR",
    )?;
    ensure_eq(
        &error.message,
        &"semaphore closed",
        "error.message == \"semaphore closed\"",
    )?;
    ensure(
        !ran.load(Ordering::SeqCst),
        "the work must not run without a permit",
    )?;
    Ok(())
}

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
                    let prev = current.fetch_add(1, Ordering::SeqCst);
                    let now = prev.checked_add(1).ok_or_else(|| {
                        ErrorData::internal_error("task counter overflowed", None)
                    })?;
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

    ensure(
        peak.load(Ordering::SeqCst) <= PERMITS,
        format!(
            "observed {} concurrent blocking operations against a {}-permit semaphore",
            peak.load(Ordering::SeqCst),
            PERMITS
        ),
    )?;
    Ok(())
}

type ProgressReader =
    tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>;
type RunningTestServer =
    rmcp::service::RunningService<rmcp::RoleServer, crate::server::SlugAuditServer>;

async fn connected_progress_peer() -> TestResult<(RunningTestServer, ProgressReader)> {
    use rmcp::ServiceExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (server_io, client_io) = tokio::io::duplex(65536);
    let server = tokio::spawn(async move {
        crate::server::SlugAuditServer::new()
            .serve(server_io)
            .await
            .check("server handshake")
    });
    let (read_half, mut write_half) = tokio::io::split(client_io);
    let mut lines = BufReader::new(read_half).lines();
    write_half
        .write_all(
            concat!(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
                "\n",
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "\n"
            )
            .as_bytes(),
        )
        .await
        .check("client handshake write")?;
    let running = server.await.check("server task")??;
    lines
        .next_line()
        .await
        .check("initialize read")?
        .check("initialize line")?;
    Ok((running, lines))
}

async fn drain_progress_notifications(
    lines: &mut ProgressReader,
) -> TestResult<Vec<(f64, String)>> {
    let mut notifications = Vec::new();
    while let Ok(next) =
        tokio::time::timeout(std::time::Duration::from_millis(80), lines.next_line()).await
    {
        let Some(line) = next.check("read notification line")? else {
            break;
        };
        let value: serde_json::Value = serde_json::from_str(&line).check("valid json")?;
        let fraction = value
            .pointer("/params/progress")
            .and_then(serde_json::Value::as_f64)
            .check("progress fraction")?;
        let message = value
            .pointer("/params/message")
            .and_then(serde_json::Value::as_str)
            .check("progress message")?
            .to_owned();
        notifications.push((fraction, message));
    }
    Ok(notifications)
}

#[tokio::test(flavor = "multi_thread")]
async fn run_blocking_delivers_monotonic_in_order_progress_without_premature_completion()
-> TestResult {
    use crate::progress::ProgressEvent;
    use crate::server_progress::{SYNC_PROGRESS_CAP, open_progress_session};
    use rmcp::model::{NumberOrString, ProgressToken};

    let (running, mut lines) = connected_progress_peer().await?;
    let peer = running.peer().clone();
    let (progress, inner_sink) =
        open_progress_session(Some((peer, ProgressToken(NumberOrString::Number(42)))));
    let late_sink = Arc::clone(&inner_sink);
    let semaphore = Arc::new(Semaphore::new(2));

    let answer = run_blocking(
        semaphore,
        "health",
        move || {
            inner_sink.emit(ProgressEvent::Started {
                phase: "ensuring_current",
            });
            inner_sink.emit(ProgressEvent::Sampling {
                phase: "publishing",
                current: 3,
                total: 4,
            });
            std::thread::sleep(std::time::Duration::from_millis(110));
            inner_sink.emit(ProgressEvent::Sampling {
                phase: "publishing",
                current: 1,
                total: 4,
            });
            inner_sink.emit(ProgressEvent::Completed {
                phase: "ensuring_current",
            });
            Ok::<_, ErrorData>("ok")
        },
        progress,
    )
    .await
    .check("run_blocking succeeds")?;
    ensure_eq(&answer, &"ok", "answer == \"ok\"")?;

    late_sink.emit(ProgressEvent::Completed { phase: "late" });

    let notifications = drain_progress_notifications(&mut lines).await?;

    let messages: Vec<&str> = notifications.iter().map(|(_, msg)| msg.as_str()).collect();
    ensure_eq(
        &messages,
        &[
            "health queued",
            "health started",
            "ensuring_current: started",
            "publishing: 3/4",
            "publishing: 1/4",
            "ensuring_current: completed",
            "health completed",
        ],
        "lifecycle and inner progress messages arrive in exact FIFO order without late stragglers",
    )?;

    for pair in notifications.windows(2) {
        let &[(prev, ref prev_msg), (next, ref next_msg)] = pair else {
            continue;
        };
        ensure(
            prev <= next,
            format!("progress regressed from {prev} ({prev_msg}) to {next} ({next_msg})"),
        )?;
    }
    let sync_done = notifications
        .iter()
        .find(|(_, msg)| msg == "ensuring_current: completed")
        .check("ensuring_current completed present")?
        .0;
    ensure(
        sync_done <= SYNC_PROGRESS_CAP,
        format!("inner sync completion must stay <= {SYNC_PROGRESS_CAP}, got {sync_done}"),
    )?;
    ensure_eq(
        &notifications.first().check("first notification")?.0,
        &0.0,
        "starts at 0.0",
    )?;
    ensure_eq(
        &notifications.last().check("last notification")?.0,
        &1.0,
        "ends at 1.0",
    )?;
    Ok(())
}
