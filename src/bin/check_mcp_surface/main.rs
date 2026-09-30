#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! MCP surface-snapshot gate.
//!
//! `tools/list` and `resources/list` are the contract a client sees before it
//! makes any call: tool names, titles, descriptions, input schemas, output
//! schemas, annotations, and the resource catalogue. Nothing in the Rust types
//! stops a description being reworded, an annotation flipping, or a resource
//! moving — and a client holding the old contract keeps acting on it. This
//! gate derives the live surface **in-process** from the router
//! (`server::tool_surface` and `resources::list`, the same values the
//! generated `list_tools` / `list_resources` return, not a reconstruction),
//! compares it against the committed snapshot, and fails on drift.
//!
//! The default refuses: recording is an explicit `--record`, in the style of
//! `check_performance --record`, so a surface change shows up as a committed
//! diff in review rather than passing silently. Deriving in-process rather
//! than spawning the server keeps the snapshot deterministic and fast.
//!
//! Pure-Rust: no Python, no shell. stdlib + `serde_json` only.

mod diff;
mod surface;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Where the committed surface lives. The planning record's archive is where
/// a gate's committed input lives (`perf_baseline.json` is the precedent) even
/// though the gate consumes it as live state.
const SNAPSHOT_PATH: &str = ".planning/archive/mcp_surface.json";

/// How many differing paths to print before summarizing the remainder. A
/// wholesale surface rewrite would otherwise print thousands of lines; the
/// total count is always printed.
const MAX_REPORTED_DIFFERENCES: usize = 40;

fn main() -> std::process::ExitCode {
    use std::path::PathBuf;

    let mut root = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("mcp-surface: cannot determine the project root: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    let mut should_record = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!(
                    "Compares the live MCP surface (tools/list + resources/list) against the \
                     committed snapshot and fails on drift.\n\
                     Usage: check_mcp_surface [--record] [--root <dir>]\n\
                     --record: rewrite {SNAPSHOT_PATH} from the live surface."
                );
                return std::process::ExitCode::SUCCESS;
            }
            "--record" => should_record = true,
            "--root" => {
                let Some(value) = args.next() else {
                    eprintln!("--root requires a project directory");
                    return std::process::ExitCode::from(2);
                };
                root = PathBuf::from(value);
            }
            other => {
                eprintln!("unknown argument: {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }

    let live = match surface::build() {
        Ok(live) => live,
        Err(error) => {
            eprintln!("mcp-surface: cannot render the live surface: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    let path = root.join(SNAPSHOT_PATH);
    if should_record {
        write_snapshot(&path, &live)
    } else {
        compare_snapshot(&path, &live)
    }
}

/// Writes the live surface over the snapshot. Only reachable via `--record`.
fn write_snapshot(path: &std::path::Path, live: &serde_json::Value) -> std::process::ExitCode {
    let mut rendered = match serde_json::to_string_pretty(live) {
        Ok(rendered) => rendered,
        Err(error) => {
            eprintln!("mcp-surface: cannot render the snapshot: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    rendered.push('\n');
    if let Err(error) = std::fs::write(path, rendered) {
        eprintln!("mcp-surface: cannot write {}: {error}", path.display());
        return std::process::ExitCode::from(2);
    }
    println!(
        "mcp-surface: recorded {} ({} tools, {} resources)",
        path.display(),
        surface::tool_count(live),
        surface::resource_count(live)
    );
    std::process::ExitCode::SUCCESS
}

/// Compares the committed snapshot against the live surface.
fn compare_snapshot(path: &std::path::Path, live: &serde_json::Value) -> std::process::ExitCode {
    let committed_text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "mcp-surface: cannot read {}: {error}; run `cargo run --bin check_mcp_surface -- \
                 --record` once the surface is what it should be",
                path.display()
            );
            return std::process::ExitCode::from(1);
        }
    };
    let committed: serde_json::Value = match serde_json::from_str(&committed_text) {
        Ok(committed) => committed,
        Err(error) => {
            eprintln!("mcp-surface: {} is not valid JSON: {error}", path.display());
            return std::process::ExitCode::from(2);
        }
    };

    let differences = diff::differences(&committed, live);
    for difference in differences.iter().take(MAX_REPORTED_DIFFERENCES) {
        println!("mcp-surface: {difference}");
    }
    if differences.len() > MAX_REPORTED_DIFFERENCES {
        println!(
            "mcp-surface: … and {} more differing paths",
            differences.len() - MAX_REPORTED_DIFFERENCES
        );
    }
    if differences.is_empty() {
        println!("mcp-surface: PASS");
        return std::process::ExitCode::SUCCESS;
    }
    println!(
        "mcp-surface: FAIL — {} differing path(s) against {}; if the change is deliberate, \
         re-run with --record",
        differences.len(),
        path.display()
    );
    std::process::ExitCode::from(1)
}
