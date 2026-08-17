//! Startup latency benchmark (Task 9.2 last unmeasured row).
//!
//! "Ready for first tool call" means a process start followed by a full
//! MCP `initialize` round-trip against the just-spawned binary — the
//! worst case the user actually pays (cold invoke → first tool answer is
//! the user-visible startup). Run as `cargo bench --bench startup`; spawns
//! the compiled `slugaudit-mcp` binary via `CARGO_BIN_EXE_slugaudit-mcp`
//! so the bench tests the same binary CI ships.
//!
//! The bench reports the median wall-clock time from `Command::spawn` to
//! the first JSON-RPC `initialize` response on stdio. It is intentionally
//! a process spawn — in-process startup would not exercise the
//! rustc-linker-elimination path the cold invoke does, so the number
//! would be unrealistically low.
use criterion::{Criterion, criterion_group, criterion_main};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Instant;

/// MCP `initialize` request — exactly the first message an MCP host sends.
/// Static string so the bench has no per-iteration allocation cost.
const INITIALIZE_REQUEST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"startup-bench","version":"0.0.0"}}}"#;

fn initialize_round_trip(binary_path: &std::path::Path) -> std::time::Duration {
    let mut child = Command::new(binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Quiet down tracing — the bench measures tool-call latency, not
        // the log volume the binary decides to print.
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("spawn slugaudit-mcp");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let mut stdout = child.stdout.take().expect("stdout pipe");

    let start = Instant::now();
    stdin
        .write_all(INITIALIZE_REQUEST.as_bytes())
        .expect("write initialize");
    drop(stdin); // close stdin so the server sees EOF after the one request

    let mut response = Vec::new();
    stdout
        .read_to_end(&mut response)
        .expect("read initialize response");
    let elapsed = start.elapsed();

    let _ = child.wait();
    assert!(
        response.starts_with(b"{\"jsonrpc\":\"2.0\""),
        "first MCP frame must be a valid response, got {} bytes starting {:?}",
        response.len(),
        response.first().map(u8::to_string).unwrap_or_default(),
    );
    elapsed
}

fn bench_startup(c: &mut Criterion) {
    // `CARGO_BIN_EXE_slugaudit-mcp` is set by `cargo bench` for benches
    // that `harness = false`; fallback to a sibling binary at
    // `target/release/slugaudit-mcp` for direct `cargo run` invocations
    // of just this file (none expected, but a missing var would otherwise
    // panic on first iteration).
    let binary_path = option_env!("CARGO_BIN_EXE_slugaudit-mcp")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("release")
                .join("slugaudit-mcp")
        });

    // Warm: the first iteration pays filesystem-cached binary load +
    // dynamic libraries; a steep cold-cost on first call. The bench
    // reports the steady-state number, matching what CI retries measure.
    let _ = initialize_round_trip(&binary_path);

    c.bench_function("startup_round_trip", |b| {
        b.iter(|| initialize_round_trip(&binary_path));
    });
}

criterion_group!(benches, bench_startup);
criterion_main!(benches);
