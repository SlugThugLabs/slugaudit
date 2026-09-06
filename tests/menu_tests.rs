//! Smoke tests for the `menu` command.

use std::io::Read as _;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MENU_TIMEOUT: Duration = Duration::from_secs(20);

fn run_menu(input: &str, extra_env: Option<(&str, &str)>) -> (ExitStatus, String, String) {
    let binary = env!("CARGO_BIN_EXE_slugaudit");
    let mut cmd = Command::new(binary);
    cmd.arg("menu");
    if let Some((key, value)) = extra_env {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn slugaudit menu");

    {
        let mut stdin = child.stdin.take().expect("menu stdin");
        for choice in input.lines() {
            use std::io::Write as _;
            writeln!(stdin, "{choice}").expect("write menu choice");
        }
    }

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

    let deadline = Instant::now() + MENU_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait menu") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("menu did not exit within {MENU_TIMEOUT:?}");
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

#[test]
fn menu_renders_and_exits_cleanly_on_the_quit_option() {
    let (status, stdout, stderr) = run_menu("6", None);
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
}

#[test]
fn menu_rejects_invalid_input_then_continues_to_exit() {
    let (status, stdout, stderr) = run_menu("999\n6", None);
    assert!(
        status.success(),
        "an invalid choice followed by quit must exit 0; stderr: {stderr}"
    );
    assert!(
        stdout.contains("Invalid choice"),
        "a bad choice must be surfaced"
    );
    assert!(stdout.contains("Choose an option"), "menu must re-prompt");
}

#[test]
fn menu_dispatches_the_other_client_instructions_step() {
    let (status, stdout, _stderr) = run_menu("4\n6", None);
    assert!(status.success(), "option 4 then quit must exit 0");
    assert!(
        stdout.contains("mcpServers"),
        "option 4 must print config snippet"
    );
    assert!(
        stdout.contains("slugaudit"),
        "snippet must name server 'slugaudit'"
    );
}

#[test]
fn menu_runs_the_install_step_into_the_slugthug_home() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().to_str().expect("temp path is utf-8");
    let (status, stdout, stderr) = run_menu("1\n6", Some(("SLUGTHUG_HOME", home)));
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
}
