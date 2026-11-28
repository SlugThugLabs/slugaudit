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

fn parse_logged_elapsed_ms(logs: &str, tool: &str) -> TestResult<u64> {
    let needle = format!("tool=\"{tool}\"");
    let line = logs
        .lines()
        .find(|line| line.contains(&needle) && line.contains("tool call completed"))
        .check("completion line captured")?;
    let (_, rest) = line
        .split_once("elapsed_ms=")
        .check("elapsed_ms field present")?;
    let ms_str = rest
        .split([' ', '}', ','])
        .next()
        .check("elapsed_ms token")?;
    ms_str.parse::<u64>().check("elapsed_ms parses as u64")
}

#[test]
fn run_blocking_excludes_semaphore_permit_wait_from_elapsed_ms() -> TestResult {
    const TOOL: &str = "probe_permit_wait_timing";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .check("test runtime")?;
    let wall_started = std::time::Instant::now();
    let (result, logs) = crate::util::test_capture::capture_at_level(
        || -> TestResult {
            runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    let semaphore = Arc::new(Semaphore::new(1));
                    let held = Arc::clone(&semaphore)
                        .acquire_owned()
                        .await
                        .check("take sole permit")?;
                    let releaser = tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                        drop(held);
                    });
                    let out = run_blocking(semaphore, TOOL, || Ok::<_, ErrorData>(()), None).await;
                    releaser.await.check("releaser joins")?;
                    out.check("run_blocking succeeds")
                })
                .await
                .check("permit-wait test timed out")?
            })
        },
        tracing::Level::INFO,
    )?;
    result?;
    let wall_elapsed = wall_started.elapsed();
    let elapsed_ms = parse_logged_elapsed_ms(&logs, TOOL)?;
    ensure(
        wall_elapsed >= std::time::Duration::from_millis(100),
        format!("test expected >= 100ms wall wait, got {wall_elapsed:?}"),
    )?;
    ensure(
        elapsed_ms < 60,
        format!("permit wait (~120ms) must be excluded from elapsed_ms, got {elapsed_ms}ms"),
    )?;
    Ok(())
}

struct DelayedWriteIo {
    inner: tokio::io::DuplexStream,
    delay_once: Arc<std::sync::atomic::AtomicBool>,
    sleep: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}

impl tokio::io::AsyncRead for DelayedWriteIo {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for DelayedWriteIo {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.sleep.is_none()
            && self
                .delay_once
                .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.sleep = Some(Box::pin(tokio::time::sleep(
                std::time::Duration::from_millis(150),
            )));
        }
        if let Some(sleep) = self.sleep.as_mut() {
            match std::future::Future::poll(sleep.as_mut(), cx) {
                std::task::Poll::Pending => return std::task::Poll::Pending,
                std::task::Poll::Ready(()) => self.sleep = None,
            }
        }
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[test]
fn run_blocking_excludes_progress_drain_wait_from_elapsed_ms() -> TestResult {
    use crate::progress::ProgressEvent;
    use crate::server_progress::open_progress_session;
    use rmcp::ServiceExt;
    use rmcp::model::{NumberOrString, ProgressToken};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const TOOL: &str = "probe_drain_wait_timing";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .check("test runtime")?;
    let wall_started = std::time::Instant::now();
    let (result, logs) = crate::util::test_capture::capture_at_level(
        || -> TestResult {
            runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    let delay_once = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let (server_duplex, client_io) = tokio::io::duplex(65536);
                    let server_io = DelayedWriteIo {
                        inner: server_duplex,
                        delay_once: Arc::clone(&delay_once),
                        sleep: None,
                    };
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

                    let peer = running.peer().clone();
                    let (progress, inner_sink) = open_progress_session(Some((
                        peer,
                        ProgressToken(NumberOrString::Number(99)),
                    )));
                    run_blocking(
                        Arc::new(Semaphore::new(2)),
                        TOOL,
                        move || {
                            delay_once.store(true, Ordering::Release);
                            inner_sink.emit(ProgressEvent::Started { phase: "step" });
                            Ok::<_, ErrorData>(())
                        },
                        progress,
                    )
                    .await
                    .check("run_blocking succeeds")?;
                    let notifications = drain_progress_notifications(&mut lines).await?;
                    ensure(
                        notifications
                            .iter()
                            .any(|(_, msg)| msg == "probe_drain_wait_timing completed"),
                        format!("completed notification must arrive: {notifications:?}"),
                    )?;
                    drop(write_half);
                    drop(running);
                    Ok(())
                })
                .await
                .check("progress-drain test timed out")?
            })
        },
        tracing::Level::INFO,
    )?;
    result?;
    let wall_elapsed = wall_started.elapsed();
    let elapsed_ms = parse_logged_elapsed_ms(&logs, TOOL)?;
    ensure(
        wall_elapsed >= std::time::Duration::from_millis(120),
        format!("test expected >= 120ms progress drain wait, got {wall_elapsed:?}"),
    )?;
    ensure(
        elapsed_ms < 60,
        format!("progress drain (~150ms) must be excluded from elapsed_ms, got {elapsed_ms}ms"),
    )?;
    Ok(())
}
