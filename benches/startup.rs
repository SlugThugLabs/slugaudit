#![forbid(unsafe_code)]

//! Startup latency benchmark.
//!
//! "Ready for first tool call" means a process start followed by a full
//! MCP `initialize` round-trip against the just-spawned binary — the
//! worst case the user actually pays (cold invoke → first tool answer is
//! the user-visible startup). Run as `cargo bench --bench startup`; spawns
//! the compiled `slugaudit` binary via `CARGO_BIN_EXE_slugaudit`
//! so the bench tests the same binary CI ships.
//!
//! The bench reports the median wall-clock time from `Command::spawn` to
//! the first JSON-RPC `initialize` response on stdio. It is intentionally
//! a process spawn — in-process startup would not exercise the
//! rustc-linker-elimination path the cold invoke does, so the number
//! would be unrealistically low.
use criterion::Criterion;
use slugaudit_test_check::{Check, TestResult};
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

#[path = "common/failure.rs"]
mod failure;

use failure::{FirstFailure, exit_code};

/// MCP `initialize` request — exactly the first message an MCP host sends.
/// Static string so the bench has no per-iteration allocation cost.
const INITIALIZE_REQUEST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"startup-bench","version":"0.0.0"}}}"#;

fn initialize_round_trip(binary_path: &std::path::Path) -> TestResult<std::time::Duration> {
    let mut child = Command::new(binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Quiet down tracing — the bench measures tool-call latency, not
        // the log volume the binary decides to print.
        .env("RUST_LOG", "warn")
        .spawn()
        .check("spawn slugaudit")?;

    let mut stdin = child.stdin.take().check("take stdin pipe")?;
    let mut stdout = child.stdout.take().check("take stdout pipe")?;

    let start = Instant::now();
    stdin
        .write_all(INITIALIZE_REQUEST.as_bytes())
        .check("write initialize")?;
    drop(stdin); // close stdin so the server sees EOF after the one request

    let mut response = Vec::new();
    stdout
        .read_to_end(&mut response)
        .check("read initialize response")?;
    let elapsed = start.elapsed();

    child.wait().check("wait for slugaudit to exit")?;
    let first_byte = match response.first() {
        Some(byte) => byte.to_string(),
        None => "none (empty response)".to_owned(),
    };
    assert!(
        response.starts_with(b"{\"jsonrpc\":\"2.0\""),
        "first MCP frame must be a valid response, got {} bytes starting {first_byte}",
        response.len(),
    );
    Ok(elapsed)
}

fn bench_startup(c: &mut Criterion) -> TestResult {
    // `CARGO_BIN_EXE_slugaudit` is set by `cargo bench` for benches
    // that `harness = false`; fallback to a sibling binary at
    // `target/release/slugaudit` for direct `cargo run` invocations
    // of just this file (none expected, but a missing var would otherwise
    // panic on first iteration).
    let binary_path = std::env::var_os("CARGO_BIN_EXE_slugaudit").map_or_else(
        || {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("release")
                .join("slugaudit")
        },
        std::path::PathBuf::from,
    );

    // Warm: the first iteration pays filesystem-cached binary load +
    // dynamic libraries; a steep cold-cost on first call. The bench
    // reports the steady-state number, matching what CI retries measure.
    initialize_round_trip(&binary_path)?;

    let failure = FirstFailure::new();
    c.bench_function("startup_round_trip", |b| {
        b.iter(|| failure.record(initialize_round_trip(&binary_path)));
    });
    failure.finish()
}

fn main() -> ExitCode {
    let mut criterion = Criterion::default().configure_from_args();
    let result = bench_startup(&mut criterion);
    criterion.final_summary();
    exit_code(result)
}
