use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type ClientLines = tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>;

async fn connected_peer_and_lines() -> TestResult<(
    rmcp::service::RunningService<RoleServer, crate::server::SlugAuditServer>,
    ClientLines,
)> {
    use rmcp::ServiceExt;

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
    Ok((running, lines))
}

async fn recv_progress(lines: &mut ClientLines) -> TestResult<(f64, String, serde_json::Value)> {
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
    let fraction = value
        .pointer("/params/progress")
        .and_then(serde_json::Value::as_f64)
        .check("notification has a numeric progress")?;
    let message = value
        .pointer("/params/message")
        .and_then(serde_json::Value::as_str)
        .check("notification has a message")?
        .to_owned();
    Ok((fraction, message, value))
}

#[test]
fn throttle_passes_first_event_and_coalesces_within_the_window() -> TestResult {
    let throttle = Throttle::new(std::time::Duration::from_millis(100));
    let t0 = std::time::Instant::now();
    let t1 = t0
        .checked_add(std::time::Duration::from_millis(1))
        .check("t0 + 1ms")?;
    let t50 = t0
        .checked_add(std::time::Duration::from_millis(50))
        .check("t0 + 50ms")?;
    let t101 = t0
        .checked_add(std::time::Duration::from_millis(101))
        .check("t0 + 101ms")?;
    let t102 = t0
        .checked_add(std::time::Duration::from_millis(102))
        .check("t0 + 102ms")?;

    ensure(
        throttle.should_emit(t0).check("clock")?,
        "first event always passes",
    )?;
    ensure(
        !throttle.should_emit(t1).check("clock")?,
        "event 1 ms in is inside the window and must be dropped",
    )?;
    ensure(
        !throttle.should_emit(t50).check("clock")?,
        "event 50 ms in is still inside the window",
    )?;
    ensure(
        throttle.should_emit(t101).check("clock")?,
        "event after the window elapses passes and resets the clock",
    )?;
    ensure(
        !throttle.should_emit(t102).check("clock")?,
        "a fresh window starts at the passed event",
    )?;
    Ok(())
}

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

#[tokio::test(flavor = "multi_thread")]
async fn progress_sink_off_runtime_thread_delivers_scaled_notification() -> TestResult {
    let (running, mut lines) = connected_peer_and_lines().await?;

    let peer = running.peer().clone();
    let (_session, sink) = open_progress_session(Some((
        peer,
        ProgressToken(rmcp::model::NumberOrString::Number(7)),
    )));
    let emitter = std::thread::spawn(move || {
        sink.emit(ProgressEvent::Sampling {
            phase: "sample",
            current: 1,
            total: 2,
        });
    });
    crate::util::join_thread(emitter).check("emit from a runtime-less thread must not panic")?;

    let (fraction, message, value) = recv_progress(&mut lines).await?;
    ensure_eq(
        value
            .pointer("/params/progressToken")
            .check("notification has a progress token")?,
        &7,
        "value[\"params\"][\"progressToken\"] == 7",
    )?;
    let expected = 0.5 * SYNC_PROGRESS_CAP;
    ensure(
        (fraction - expected).abs() < 1e-12,
        format!("expected {expected} progress fraction, got {fraction}"),
    )?;
    ensure_eq(&message, &"sample: 1/2", "message == \"sample: 1/2\"")?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_sink_suppresses_notifications_after_session_is_dropped() -> TestResult {
    let (running, mut lines) = connected_peer_and_lines().await?;

    let peer = running.peer().clone();
    let (session, sink) = open_progress_session(Some((
        peer,
        ProgressToken(rmcp::model::NumberOrString::Number(9)),
    )));
    drop(session);
    sink.emit(ProgressEvent::Started { phase: "late" });
    sink.emit(ProgressEvent::Completed { phase: "late" });

    let received =
        tokio::time::timeout(std::time::Duration::from_millis(100), lines.next_line()).await;
    ensure(
        received.is_err(),
        "no stale progress notification may arrive once the session closes",
    )?;
    Ok(())
}

/// Sync-phase `Completed` events cap at `SYNC_PROGRESS_CAP` (`0.9`) so the
/// server never claims `1.0` completion while the tool's own read or scan is
/// still running; only `ProgressSession::finish` emits `1.0`.
#[tokio::test(flavor = "multi_thread")]
async fn sync_completed_caps_below_one_and_only_finish_emits_one() -> TestResult {
    let (running, mut lines) = connected_peer_and_lines().await?;

    let peer = running.peer().clone();
    let (session, sink) = open_progress_session(Some((
        peer,
        ProgressToken(rmcp::model::NumberOrString::Number(10)),
    )));
    let session = session.check("progress session opened")?;

    sink.emit(ProgressEvent::Sampling {
        phase: "publishing",
        current: 2,
        total: 2,
    });
    sink.emit(ProgressEvent::Completed {
        phase: "publishing",
    });
    sink.emit(ProgressEvent::Completed {
        phase: "ensuring_current",
    });
    session.finish("report").await;

    let (sample_frac, sample_msg, _) = recv_progress(&mut lines).await?;
    let (pub_frac, pub_msg, _) = recv_progress(&mut lines).await?;
    let (sync_frac, sync_msg, _) = recv_progress(&mut lines).await?;
    let (done_frac, done_msg, _) = recv_progress(&mut lines).await?;

    ensure_eq(&sample_msg, &"publishing: 2/2", "sampling message")?;
    ensure(
        sample_frac < 1.0 && (sample_frac - SYNC_PROGRESS_CAP).abs() < 1e-12,
        format!("100% sampling caps at {SYNC_PROGRESS_CAP}, got {sample_frac}"),
    )?;
    ensure_eq(&pub_msg, &"publishing: completed", "publishing completed")?;
    ensure(
        pub_frac < 1.0 && (pub_frac - SYNC_PROGRESS_CAP).abs() < 1e-12,
        format!("inner publishing completed caps at {SYNC_PROGRESS_CAP}, got {pub_frac}"),
    )?;
    ensure_eq(
        &sync_msg,
        &"ensuring_current: completed",
        "ensuring_current completed",
    )?;
    ensure(
        sync_frac < 1.0 && (sync_frac - SYNC_PROGRESS_CAP).abs() < 1e-12,
        format!("ensuring_current completed caps at {SYNC_PROGRESS_CAP}, got {sync_frac}"),
    )?;
    ensure_eq(&done_msg, &"report completed", "final tool completion")?;
    ensure(
        (done_frac - 1.0).abs() < f64::EPSILON,
        format!("session.finish emits 1.0, got {done_frac}"),
    )?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_session_delivers_strictly_in_order_and_monotonic_fractions() -> TestResult {
    let (running, mut lines) = connected_peer_and_lines().await?;

    let peer = running.peer().clone();
    let (session, sink) = open_progress_session(Some((
        peer,
        ProgressToken(rmcp::model::NumberOrString::Number(11)),
    )));
    let session = session.check("progress session opened")?;

    session.send(0.0, "probe queued".to_owned());
    session.send(0.0, "probe started".to_owned());
    sink.emit(ProgressEvent::Started {
        phase: "ensuring_current",
    });
    session.send(0.6, "publishing: 3/5".to_owned());
    session.send(0.2, "publishing: 1/5".to_owned());
    sink.emit(ProgressEvent::Started { phase: "subphase" });
    session.send(0.8, "publishing: 4/5".to_owned());
    session.finish("probe").await;
    sink.emit(ProgressEvent::Completed { phase: "straggler" });

    let expected_messages = [
        "probe queued",
        "probe started",
        "ensuring_current: started",
        "publishing: 3/5",
        "publishing: 1/5",
        "subphase: started",
        "publishing: 4/5",
        "probe completed",
    ];
    let mut fractions = Vec::with_capacity(expected_messages.len());
    let mut messages = Vec::with_capacity(expected_messages.len());
    for _ in 0..expected_messages.len() {
        let (fraction, message, _) = recv_progress(&mut lines).await?;
        fractions.push(fraction);
        messages.push(message);
    }

    ensure_eq(
        &messages,
        &expected_messages,
        "notifications must arrive in exact FIFO emission order",
    )?;
    for pair in fractions.windows(2) {
        let &[prev, next] = pair else {
            continue;
        };
        ensure(
            prev <= next,
            format!("progress regressed from {prev} to {next}: {fractions:?}"),
        )?;
    }
    ensure_eq(
        fractions.first().check("first fraction")?,
        &0.0,
        "progress starts at 0.0",
    )?;
    ensure_eq(
        fractions.last().check("final fraction")?,
        &1.0,
        "progress ends at 1.0",
    )?;

    let straggler =
        tokio::time::timeout(std::time::Duration::from_millis(80), lines.next_line()).await;
    ensure(
        straggler.is_err(),
        "zero notifications may arrive after session.finish returns",
    )?;
    Ok(())
}
