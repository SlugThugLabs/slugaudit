use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

/// The sampling throttle must pass the first event, drop everything inside
/// the window, and pass again once the window elapses — the exact
/// behavior that keeps a 60 k-file import from spawning 60 k notification
/// tasks.
#[test]
fn throttle_passes_first_event_and_coalesces_within_the_window() -> TestResult {
    let throttle = Throttle::new(std::time::Duration::from_millis(100));
    let t0 = std::time::Instant::now();

    ensure(
        throttle.should_emit(t0).check("clock")?,
        "first event always passes",
    )?;
    ensure(
        !throttle
            .should_emit(t0 + std::time::Duration::from_millis(1))
            .check("clock")?,
        "event 1 ms in is inside the window and must be dropped",
    )?;
    ensure(
        !throttle
            .should_emit(t0 + std::time::Duration::from_millis(50))
            .check("clock")?,
        "event 50 ms in is still inside the window",
    )?;
    ensure(
        throttle
            .should_emit(t0 + std::time::Duration::from_millis(101))
            .check("clock")?,
        "event after the window elapses passes and resets the clock",
    )?;
    ensure(
        !throttle
            .should_emit(t0 + std::time::Duration::from_millis(102))
            .check("clock")?,
        "a fresh window starts at the passed event",
    )?;
    Ok(())
}

/// A window too long for `u64` milliseconds is an error, not a clamped window.
#[test]
fn a_window_too_long_for_milliseconds_is_a_clock_error() -> TestResult {
    let throttle = Throttle::new(std::time::Duration::MAX);
    let error = throttle
        .should_emit(std::time::Instant::now())
        .check_err("the window does not fit u64 milliseconds")?;
    ensure(
        matches!(error, crate::clock::ClockError::DurationMillis(_)),
        "the failure names the millisecond conversion",
    )?;
    Ok(())
}

/// `Sampling` events come from `std::thread::scope` workers with no tokio
/// context; the sink must spawn on the runtime captured at construction.
#[tokio::test(flavor = "multi_thread")]
async fn progress_sink_off_runtime_thread_delivers_notification() -> TestResult {
    use rmcp::ServiceExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (server_io, client_io) = tokio::io::duplex(65536);
    let server = tokio::spawn(async move {
        crate::server::SlugAuditServer::new()
            .serve(server_io)
            .await
            .check("server must complete the handshake")
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
        .check("client write")?;
    let running = server.await.check("server task")??;
    lines
        .next_line()
        .await
        .check("read initialize response")?
        .check("initialize response line")?;

    let completed = Arc::new(AtomicBool::new(false));
    let sink = build_inner_sink(
        Some((
            running.peer().clone(),
            ProgressToken(rmcp::model::NumberOrString::Number(7)),
        )),
        Arc::clone(&completed),
    );
    let emitter = std::thread::spawn(move || {
        sink.emit(ProgressEvent::Sampling {
            phase: "sample",
            current: 1,
            total: 2,
        });
    });
    crate::util::join_thread(emitter).check("emit from a runtime-less thread must not panic")?;

    let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
        .await
        .check("notification within 5s")?
        .check("read notification")?
        .check("notification line")?;
    let value: serde_json::Value = serde_json::from_str(&line).check("json")?;
    ensure_eq(
        value.get("method").check("notification has a method")?,
        &"notifications/progress",
        "value[\"method\"] == \"notifications/progress\"",
    )?;
    ensure_eq(
        value
            .pointer("/params/progressToken")
            .check("notification has a progress token")?,
        &7,
        "value[\"params\"][\"progressToken\"] == 7",
    )?;
    ensure_eq(
        value
            .pointer("/params/message")
            .check("notification has a message")?,
        &"sample: 1/2",
        "value[\"params\"][\"message\"] == \"sample: 1/2\"",
    )?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_sink_suppresses_notifications_after_completion_flag_is_set() -> TestResult {
    use rmcp::ServiceExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (server_io, client_io) = tokio::io::duplex(65536);
    let server = tokio::spawn(async move {
        crate::server::SlugAuditServer::new()
            .serve(server_io)
            .await
            .check("server must complete the handshake")
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
        .check("client write")?;
    let running = server.await.check("server task")??;
    lines
        .next_line()
        .await
        .check("read initialize response")?
        .check("initialize response line")?;

    let completed = Arc::new(AtomicBool::new(true));
    let sink = build_inner_sink(
        Some((
            running.peer().clone(),
            ProgressToken(rmcp::model::NumberOrString::Number(9)),
        )),
        completed,
    );
    sink.emit(ProgressEvent::Started { phase: "late" });
    sink.emit(ProgressEvent::Completed { phase: "late" });

    let received =
        tokio::time::timeout(std::time::Duration::from_millis(100), lines.next_line()).await;
    ensure(
        received.is_err(),
        "no stale progress notification may arrive once completed is true",
    )?;
    Ok(())
}
