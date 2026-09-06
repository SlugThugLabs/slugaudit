//! Smoke tests for the `menu` command.
//!
//! These spawn the compiled binary with `menu` and drive it over a real
//! stdin pipe, so the full render → read-choice → dispatch → loop path is
//! exercised end to end against the actual executable — the in-crate tests
//! in `menu::tests` only cover pure helpers, and this is the only place
//! the real menu loop, its I/O, and its `Command` dispatch are verified.
//!
//! Side-effect hygiene — the menu is human-facing glue over `install` and
//! `connect`, so some choices mutate the machine:
//!   * Options 5 (Exit) and 3 (other-client instructions) are pure.
//!   * Option 1 (install) is pointed at a throwaway `SLUGTHUG_HOME`, so
//!     the only file created is a disposable install directory.
//!   * Options 2 (connect) and 4 (serve) are deliberately NOT driven here:
//!     `connect` writes to a real agent's config when that agent's CLI is
//!     on PATH (see the note in `tests/connect_tests.rs`), and `serve`
//!     blocks on the MCP transport. Both are exercised by their own
//!     integration tests (`tests/connect_tests.rs`, `tests/stdio_protocol.rs`).
//!
//! Because stdin is a pipe (never a terminal), `Style::stdout()` renders
//! plain, so these also pin that menu output carries no ANSI escapes when
//! redirected — corollary of the color work in `menu.rs`.

use std::io::Read as _;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MENU_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs `slugaudit-mcp menu` with the given lines fed to stdin (one choice
/// per line), waiting at most `MENU_TIMEOUT` for it to finish. Returns the
/// exit status plus the captured stdout/stderr. Fails the test, rather
/// than hanging it, if the menu does not exit in time — guarding against a
/// choice branch that blocks on MCP or loops forever on stdin EOF.
fn run_menu(input: &str, extra_env: Option<(&str, &str)>) -> (ExitStatus, String, String) {
    let binary = env!("CARGO_BIN_EXE_slugaudit-mcp");
    let mut cmd = Command::new(binary);
    cmd.arg("menu");
    if let Some((key, value)) = extra_env {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn slugaudit-mcp menu");

    // Feed the choices, then drop stdin so an already-finished menu sees
    // EOF rather than waiting on us for the next line.
    {
        let mut stdin = child.stdin.take().expect("menu stdin");
        for choice in input.lines() {
            use std::io::Write as _;
            writeln!(stdin, "{choice}").expect("write menu choice");
        }
    }

    // Drain stdout/stderr on threads so the pipe never fills up and blocks
    // the child, and so each is fully captured when we finish.
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    let mut stdout = child.stdout.take().expect("menu stdout");
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = out_tx.send(text);
    });
    let mut stderr = child.stderr.take().expect("menu stderr");
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        let _ = err_tx.send(text);
    });

    // Poll for exit with a hard deadline so a blocking regression fails
    // fast instead of stalling the whole test suite.
    let deadline = Instant::now() + MENU_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait menu") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "menu did not exit within {MENU_TIMEOUT:?}; a choice branch is blocking or looping on EOF"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    let stdout = out_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_default();
    let stderr = err_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_default();
    (status, stdout, stderr)
}

/// The plain quit path: choosing 5 must exit 0 after rendering the menu,
/// with no ANSI escapes in the (redirected) output.
#[test]
fn menu_renders_and_exits_cleanly_on_the_quit_option() {
    let (status, stdout, stderr) = run_menu("5", None);
    assert!(
        status.success(),
        "choosing 5 (Exit) must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("SlugAudit setup"),
        "menu must render its title"
    );
    assert!(
        stdout.contains("Choose an option"),
        "menu must show the prompt"
    );
    assert!(
        !stdout.contains('\u{1b}'),
        "piped stdin means output must be plain, with no ANSI escapes"
    );
}

/// An out-of-range choice must be reported as invalid, then the menu must
/// keep looping and accept the next (valid) choice rather than crash or
/// hang.
#[test]
fn menu_rejects_invalid_input_then_continues_to_exit() {
    let (status, stdout, stderr) = run_menu("999\n5", None);
    assert!(
        status.success(),
        "an invalid choice followed by quit must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("Invalid choice"),
        "a bad choice must be surfaced, got: {stdout}"
    );
    assert!(stdout.contains("Choose an option"), "menu must re-prompt");
}

/// Option 3 (other MCP client) is a pure print step; driving it must emit
/// the config snippet and return to the loop without any side effects.
#[test]
fn menu_dispatches_the_other_client_instructions_step() {
    let (status, stdout, _stderr) = run_menu("3\n5", None);
    assert!(status.success(), "option 3 then quit must exit 0");
    assert!(
        stdout.contains("mcpServers"),
        "option 3 must print the MCP config snippet"
    );
    assert!(
        stdout.contains("slugaudit"),
        "the snippet must name the server 'slugaudit'"
    );
}

/// Option 1 (install) must dispatch to the real install path and write the
/// binary into the given `SLUGTHUG_HOME` — a throwaway temp dir here, so
/// the only artifact is disposable and the test provably ran the step.
#[test]
fn menu_runs_the_install_step_into_the_slugthug_home() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().to_str().expect("temp path is utf-8");
    let (status, stdout, stderr) = run_menu("1\n5", Some(("SLUGTHUG_HOME", home)));
    assert!(
        status.success(),
        "option 1 (install) then quit must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("Installed slugaudit-mcp to"),
        "the install step must confirm its destination, got: {stdout}"
    );
    let installed = temp.path().join("slugaudit").join("slugaudit-mcp");
    assert!(
        installed.exists(),
        "option 1 must have written the binary at {}",
        installed.display()
    );
}
