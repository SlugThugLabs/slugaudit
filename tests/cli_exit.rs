#![forbid(unsafe_code)]

//! Process-level exit-code contract of the `slugaudit` binary and hermetic
//! end-to-end tests for `install.sh`.

use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn slugaudit(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_slugaudit"));
    command.args(args);
    command
}

fn run(args: &[&str]) -> TestResult<Output> {
    slugaudit(args)
        .stdin(Stdio::null())
        .output()
        .check("run slugaudit")
}

#[test]
fn cli_exit_unknown_flag_exits_2_with_a_display_message() -> TestResult {
    let output = run(&["--bogus"])?;
    let stderr = String::from_utf8(output.stderr).check("utf-8 stderr")?;
    ensure_eq(
        &output.status.code(),
        &Some(2),
        "unknown flag exits with usage code 2",
    )?;
    ensure(
        stderr.starts_with("Error: unknown command \"--bogus\""),
        &stderr,
    )?;
    Ok(())
}

#[test]
fn cli_exit_lowercase_v_prints_the_version_and_exits_0() -> TestResult {
    let output = run(&["-v"])?;
    let stdout = String::from_utf8(output.stdout).check("utf-8 stdout")?;
    ensure_eq(&output.status.code(), &Some(0), "-v exits with code 0")?;
    ensure_eq(
        &stdout,
        &format!("slugaudit {}\n", env!("CARGO_PKG_VERSION")),
        "-v prints exact package version",
    )?;
    Ok(())
}

#[test]
fn cli_exit_connect_with_an_extra_argument_exits_2() -> TestResult {
    let output = run(&["connect", "claude", "extra"])?;
    ensure_eq(
        &output.status.code(),
        &Some(2),
        "extra argument to connect exits with code 2",
    )?;
    let stderr = String::from_utf8(output.stderr).check("utf-8 stderr")?;
    ensure(stderr.contains("unexpected argument \"extra\""), &stderr)?;
    Ok(())
}

#[test]
fn cli_exit_stdin_closed_before_initialize_exits_0_with_one_clean_line() -> TestResult {
    let output = run(&[])?;
    let stderr = String::from_utf8(output.stderr).check("utf-8 stderr")?;
    ensure_eq(&output.status.code(), &Some(0), &stderr)?;
    ensure_eq(&stderr.lines().count(), &1, &stderr)?;
    ensure(!stderr.contains("ConnectionClosed("), &stderr)?;
    Ok(())
}

#[test]
fn cli_exit_sigterm_to_a_serving_process_exits_0() -> TestResult {
    let mut child = slugaudit(&[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .check("spawn slugaudit")?;
    let mut stdin = child.stdin.take().check("stdin pipe")?;
    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    writeln!(stdin, "{initialize}").check("write initialize")?;
    let mut stdout = BufReader::new(child.stdout.take().check("stdout pipe")?);
    let mut reply = String::new();
    stdout
        .read_line(&mut reply)
        .check("read initialize reply")?;
    ensure(reply.contains("\"result\""), &reply)?;
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .check("write initialized")?;
    stdin.flush().check("flush stdin")?;
    std::thread::sleep(Duration::from_millis(500));
    let kill = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .check("run kill")?;
    ensure(kill.success(), "kill -TERM succeeds")?;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(5))
        .check("SIGTERM deadline")?;
    let status = loop {
        if let Some(status) = child.try_wait().check("poll slugaudit")? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let Some(status) = status else {
        child.kill().check("kill hung slugaudit")?;
        child.wait().check("reap hung slugaudit")?;
        return Err("slugaudit did not exit within 5 s of SIGTERM with stdin open".to_owned())
            .check("unexpected outcome");
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .check("stderr pipe")?
        .read_to_string(&mut stderr)
        .check("read stderr")?;
    ensure_eq(&status.code(), &Some(0), &stderr)?;
    ensure(stderr.contains("received SIGTERM"), &stderr)?;
    drop(stdin);
    Ok(())
}

#[test]
fn every_subcommand_help_prints_its_usage_and_exits_0_without_running_it() -> TestResult {
    let home = tempfile::tempdir().check("temp home")?;
    let empty_path = tempfile::tempdir().check("empty PATH dir")?;
    for name in [
        "serve",
        "connect",
        "disconnect",
        "remove",
        "install",
        "menu",
        "update",
    ] {
        for args in [
            vec![name, "--help"],
            vec![name, "x", "--help"],
            vec![name, "-h"],
        ] {
            let output = slugaudit(&args)
                .env("HOME", home.path())
                .env("SLUGTHUG_HOME", home.path())
                .env("SLUGTHUG_CONFIG_HOME", home.path())
                .env("PATH", empty_path.path())
                .stdin(Stdio::null())
                .output()
                .check("run slugaudit")?;
            let stdout = String::from_utf8(output.stdout).check("utf-8 stdout")?;
            ensure_eq(&output.status.code(), &Some(0), format!("{args:?}"))?;
            ensure(
                stdout.starts_with("USAGE:\n    slugaudit "),
                format!("{args:?}: {stdout}"),
            )?;
            ensure(output.stderr.is_empty(), format!("{args:?}"))?;
        }
    }
    let leftovers = std::fs::read_dir(home.path()).check("read home")?.count();
    ensure_eq(&leftovers, &0, "help must not touch the home directory")?;
    let config_dir = std::fs::read_dir(home.path().join(".slugthug"))
        .check_err("help must not create the config directory")?;
    ensure_eq(
        &config_dir.kind(),
        &std::io::ErrorKind::NotFound,
        ".slugthug directory remains absent",
    )?;
    Ok(())
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, body).check("write executable script")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .check("chmod executable script")
}

#[cfg(unix)]
fn install_script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh")
}

#[cfg(unix)]
#[test]
fn install_sh_help_and_unknown_flags_behave_without_touching_home() -> TestResult {
    let sandbox = tempfile::tempdir().check("temp dir")?;
    let home = sandbox.path().join("home");
    std::fs::create_dir_all(&home).check("mkdir home")?;
    let script = install_script_path();

    let help = Command::new("bash")
        .arg(&script)
        .arg("--help")
        .env("HOME", &home)
        .output()
        .check("run install.sh --help")?;
    ensure_eq(&help.status.code(), &Some(0), "--help exits 0")?;
    let help_out = String::from_utf8_lossy(&help.stdout);
    ensure(
        help_out.contains("Usage: install.sh") && help_out.contains("--from-source"),
        format!("--help documents --from-source: {help_out}"),
    )?;

    let bad = Command::new("bash")
        .arg(&script)
        .arg("--unknown-flag")
        .env("HOME", &home)
        .output()
        .check("run install.sh --unknown-flag")?;
    ensure_eq(&bad.status.code(), &Some(2), "unknown flag exits 2")?;
    ensure(
        String::from_utf8_lossy(&bad.stderr).contains("unknown argument: --unknown-flag"),
        "stderr names unknown flag",
    )?;
    ensure(
        !home.join(".slugthug").exists(),
        "help and flag errors must not create ~/.slugthug",
    )?;
    Ok(())
}

#[cfg(unix)]
struct InstallShSandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    cwd: PathBuf,
    bin_dir: PathBuf,
    payload_file: PathBuf,
    sums_override: PathBuf,
    curl_log: PathBuf,
    cargo_marker: PathBuf,
    path_env: String,
}

#[cfg(unix)]
fn setup_install_sh_sandbox(payload: &str) -> TestResult<InstallShSandbox> {
    let dir = tempfile::tempdir().check("temp dir")?;
    let home = dir.path().join("home");
    let cwd = dir.path().join("cwd");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&home).check("mkdir home")?;
    std::fs::create_dir_all(&cwd).check("mkdir cwd")?;
    std::fs::create_dir_all(&bin_dir).check("mkdir bin")?;

    // Plant a Cargo.toml in CWD and a trap `cargo` on PATH: running `install.sh`
    // with no flags must NOT invoke `cargo build`.
    std::fs::write(
        cwd.join("Cargo.toml"),
        "[package]\nname = \"slugaudit-mcp-rust\"\nversion = \"1.1.0\"\n",
    )
    .check("write trap Cargo.toml")?;
    let cargo_marker = dir.path().join("cargo-invoked");
    write_executable(
        &bin_dir.join("cargo"),
        &format!(
            "#!/bin/sh\necho \"$*\" > '{}'\nexit 99\n",
            cargo_marker.display()
        ),
    )?;
    write_executable(
        &bin_dir.join("uname"),
        "#!/bin/sh\nif [ \"$1\" = \"-s\" ]; then echo Linux; else echo x86_64; fi\n",
    )?;

    let payload_file = dir.path().join("payload");
    std::fs::write(&payload_file, payload).check("write payload")?;
    let sha_out = Command::new("sha256sum")
        .arg(&payload_file)
        .output()
        .check("compute payload sha256")?;
    let good_sha = String::from_utf8_lossy(&sha_out.stdout)
        .split_whitespace()
        .next()
        .check("sha256 digest token")?
        .to_owned();

    let curl_log = dir.path().join("curl.log");
    let sums_override = dir.path().join("sums.txt");
    std::fs::write(
        &sums_override,
        format!("{good_sha}  slugaudit-x86_64-unknown-linux-gnu\n"),
    )
    .check("write valid SHA256SUMS")?;

    write_executable(
        &bin_dir.join("curl"),
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" != \"-q\" ]; then echo 'missing leading -q' >&2; exit 55; fi\n\
             printf '%s\\n' \"$*\" >> '{log}'\n\
             dest=\n\
             url=\n\
             while [ $# -gt 0 ]; do\n\
               if [ \"$1\" = \"-o\" ]; then dest=$2; shift 2; continue; fi\n\
               url=$1; shift\n\
             done\n\
             case \"$url\" in\n\
               */releases/latest) printf '%s' '{{\"tag_name\":\"v1.2.3\"}}' ;;\n\
               */releases/download/v1.2.3/SHA256SUMS) cp '{sums}' \"$dest\" ;;\n\
               */releases/download/v1.2.3/slugaudit-x86_64-unknown-linux-gnu) cp '{payload_path}' \"$dest\" ;;\n\
               *) echo \"unexpected url: $url\" >&2; exit 44 ;;\n\
             esac\n",
            log = curl_log.display(),
            sums = sums_override.display(),
            payload_path = payload_file.display(),
        ),
    )?;
    let path_env = format!("{}:/usr/bin:/bin", bin_dir.display());
    Ok(InstallShSandbox {
        _dir: dir,
        home,
        cwd,
        bin_dir,
        payload_file,
        sums_override,
        curl_log,
        cargo_marker,
        path_env,
    })
}

#[cfg(unix)]
fn verify_install_sh_from_source(env: &InstallShSandbox, expected_prev: &str) -> TestResult {
    write_executable(
        &env.bin_dir.join("cargo"),
        &format!(
            "#!/bin/sh\n\
             printf '%s|%s|%s\\n' \"$TSLP_LINK_MODE\" \"$TSLP_LANGUAGES\" \"$*\" > '{marker}'\n\
             mkdir -p target/release\n\
             printf '#!/bin/sh\\necho built-from-source\\n' > target/release/slugaudit\n",
            marker = env.cargo_marker.display()
        ),
    )?;
    let from_source = Command::new("bash")
        .arg(install_script_path())
        .arg("--from-source")
        .current_dir(&env.cwd)
        .env("HOME", &env.home)
        .env("PATH", &env.path_env)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .check("run install.sh --from-source")?;
    ensure(
        from_source.status.success(),
        format!(
            "--from-source failed: {}",
            String::from_utf8_lossy(&from_source.stderr)
        ),
    )?;
    ensure_eq(
        &std::fs::read_to_string(&env.cargo_marker)
            .check("read cargo args")?
            .trim(),
        &"static|rust|build --release --locked --bin slugaudit",
        "--from-source passes TSLP_LINK_MODE=static TSLP_LANGUAGES=rust to cargo build",
    )?;
    ensure_eq(
        &std::fs::read_to_string(env.home.join(".slugthug/slugaudit/slugaudit.prev"))
            .check("read retained .prev")?,
        &expected_prev,
        "retains previous release binary at slugaudit.prev",
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn install_sh_downloads_pinned_release_tag_ignores_cwd_cargo_toml_and_checks_sha256() -> TestResult
{
    const PAYLOAD: &str = "#!/bin/sh\necho slugaudit-release-1.2.3\n";
    let env = setup_install_sh_sandbox(PAYLOAD)?;
    let script = install_script_path();
    let ok = Command::new("bash")
        .arg(&script)
        .current_dir(&env.cwd)
        .env("HOME", &env.home)
        .env("PATH", &env.path_env)
        .output()
        .check("run install.sh release install")?;
    ensure(
        ok.status.success(),
        format!(
            "install.sh failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&ok.stdout),
            String::from_utf8_lossy(&ok.stderr)
        ),
    )?;
    ensure(
        !env.cargo_marker.exists(),
        "default install.sh must ignore ./Cargo.toml unless --from-source is passed",
    )?;
    let target = env.home.join(".slugthug/slugaudit/slugaudit");
    ensure_eq(
        &std::fs::read_to_string(&target).check("read installed binary")?,
        &PAYLOAD,
        "installed binary matches verified release payload",
    )?;
    let logged_curl = std::fs::read_to_string(&env.curl_log).check("read curl log")?;
    ensure(
        logged_curl.contains("--proto-redir =https")
            && logged_curl.contains("/releases/download/v1.2.3/slugaudit-x86_64-unknown-linux-gnu")
            && logged_curl.contains("/releases/download/v1.2.3/SHA256SUMS"),
        format!(
            "both downloads must use pinned tag v1.2.3 and --proto-redir =https: {logged_curl}"
        ),
    )?;

    // Corrupt SHA256SUMS and confirm install.sh refuses to overwrite `target`.
    std::fs::write(
        &env.sums_override,
        format!("{}  slugaudit-x86_64-unknown-linux-gnu\n", "0".repeat(64)),
    )
    .check("write mismatching SHA256SUMS")?;
    std::fs::write(&env.payload_file, "tampered-payload").check("write tampered payload")?;
    let mismatch = Command::new("bash")
        .arg(&script)
        .current_dir(&env.cwd)
        .env("HOME", &env.home)
        .env("PATH", &env.path_env)
        .output()
        .check("run install.sh with mismatching sha256")?;
    ensure(
        !mismatch.status.success(),
        "checksum mismatch must fail install.sh",
    )?;
    ensure(
        String::from_utf8_lossy(&mismatch.stderr).contains("SHA256 checksum verification failed"),
        "stderr reports SHA256 verification failure",
    )?;
    ensure_eq(
        &std::fs::read_to_string(&target).check("read preserved binary")?,
        &PAYLOAD,
        "failed checksum leaves previously installed binary untouched",
    )?;

    verify_install_sh_from_source(&env, PAYLOAD)
}
