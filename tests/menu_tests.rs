#![forbid(unsafe_code)]

//! Smoke tests for the `menu` command.

use slugaudit_test_check::{Check, TestResult};
use std::io::Read as _;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MENU_TIMEOUT: Duration = Duration::from_secs(20);

fn run_menu(
    input: &str,
    extra_env: Option<(&str, &str)>,
) -> TestResult<(ExitStatus, String, String)> {
    let binary = env!("CARGO_BIN_EXE_slugaudit");
    // The menu reaches `connect`/`disconnect` on options 2 and 3, and those
    // resolve and execute installed client CLIs (`mcp add`/`mcp remove`)
    // and read client configs from the user's home. Even though no test here
    // chooses those options, pin every home into a temp dir and replace
    // `PATH` with an empty directory, so no option can ever reach a real
    // client or rewrite a live config such as `~/.codex/config.toml`.
    let sandbox = tempfile::tempdir().check("create temp dir")?;
    let home = sandbox.path().join("home");
    let empty_path = sandbox.path().join("bin");
    std::fs::create_dir_all(&home).check("mkdir home")?;
    std::fs::create_dir_all(&empty_path).check("mkdir empty path")?;

    let mut cmd = Command::new(binary);
    cmd.arg("menu")
        .env("HOME", &home)
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &empty_path);
    if let Some((key, value)) = extra_env {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().check("spawn slugaudit menu")?;

    {
        let mut stdin = child.stdin.take().check("take menu stdin")?;
        for choice in input.lines() {
            use std::io::Write as _;
            writeln!(stdin, "{choice}").check("write menu choice")?;
        }
    }

    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    let mut stdout = child.stdout.take().check("take menu stdout")?;
    std::thread::spawn(move || {
        let mut text = String::new();
        let read = stdout.read_to_string(&mut text).map(|_| text);
        // The receiver is gone only after the timeout path below already
        // failed the test; report the lost capture instead of dropping it.
        if let Err(error) = out_tx.send(read) {
            eprintln!("menu stdout capture had no receiver: {error}");
        }
    });
    let mut stderr = child.stderr.take().check("take menu stderr")?;
    std::thread::spawn(move || {
        let mut text = String::new();
        let read = stderr.read_to_string(&mut text).map(|_| text);
        if let Err(error) = err_tx.send(read) {
            eprintln!("menu stderr capture had no receiver: {error}");
        }
    });

    let deadline = Instant::now() + MENU_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().check("poll menu exit status")? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().check("kill the hung menu")?;
            child.wait().check("reap the hung menu")?;
            return Err(format!("menu did not exit within {MENU_TIMEOUT:?}"))
                .check("unexpected outcome");
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    let stdout = out_rx
        .recv_timeout(Duration::from_secs(5))
        .check("menu stdout capture must finish")?
        .check("menu stdout must be readable")?;
    let stderr = err_rx
        .recv_timeout(Duration::from_secs(5))
        .check("menu stderr capture must finish")?
        .check("menu stderr must be readable")?;
    Ok((status, stdout, stderr))
}

#[test]
fn menu_renders_and_exits_cleanly_on_the_quit_option() -> TestResult {
    let (status, stdout, stderr) = run_menu("6", None)?;
    assert!(
        status.success(),
        "choosing 6 (Exit) must exit 0; stderr: {stderr}"
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
        "piped stdin means output must be plain"
    );
    Ok(())
}

#[test]
fn menu_rejects_invalid_input_then_continues_to_exit() -> TestResult {
    let (status, stdout, stderr) = run_menu("999\n6", None)?;
    assert!(
        status.success(),
        "an invalid choice followed by quit must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("Invalid choice"),
        "a bad choice must be surfaced"
    );
    assert!(stdout.contains("Choose an option"), "menu must re-prompt");
    Ok(())
}

#[test]
fn menu_dispatches_the_other_client_instructions_step() -> TestResult {
    let (status, stdout, _stderr) = run_menu("4\n6", None)?;
    assert!(status.success(), "option 4 then quit must exit 0");
    assert!(
        stdout.contains("mcpServers"),
        "option 4 must print config snippet"
    );
    assert!(
        stdout.contains("slugaudit"),
        "snippet must name server 'slugaudit'"
    );
    Ok(())
}

/// A stdin that cannot be read at all is a propagated I/O error, not an
/// endless menu loop.
#[test]
fn menu_reports_a_stdin_read_failure() -> TestResult {
    let binary = env!("CARGO_BIN_EXE_slugaudit");
    let sandbox = tempfile::tempdir().check("create temp dir")?;
    let home = sandbox.path().join("home");
    let empty_path = sandbox.path().join("bin");
    std::fs::create_dir_all(&home).check("mkdir home")?;
    std::fs::create_dir_all(&empty_path).check("mkdir empty path")?;
    // A directory opens as stdin but reading it fails with EISDIR.
    let unreadable = std::fs::File::open(sandbox.path()).check("open directory as stdin")?;

    let output = Command::new(binary)
        .arg("menu")
        .env("HOME", &home)
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &empty_path)
        .stdin(Stdio::from(unreadable))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .check("spawn slugaudit menu")?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a propagated I/O error must exit 1, not panic: {stderr}"
    );
    assert!(
        stderr.contains("Is a directory") || stderr.contains("IsADirectory"),
        "the I/O cause must be reported: {stderr}"
    );
    Ok(())
}

#[test]
fn menu_runs_the_install_step_into_the_slugthug_home() -> TestResult {
    let temp = tempfile::tempdir().check("create temp dir")?;
    let home = temp.path().to_str().check("temp path is utf-8")?;
    let (status, stdout, stderr) = run_menu("1\n6", Some(("SLUGTHUG_HOME", home)))?;
    assert!(
        status.success(),
        "option 1 (install) then quit must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("Installed slugaudit to"),
        "install step must confirm destination"
    );
    let installed = temp.path().join("slugaudit").join("slugaudit");
    assert!(
        installed.exists(),
        "option 1 must have written binary at {}",
        installed.display()
    );
    Ok(())
}
