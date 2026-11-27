#![forbid(unsafe_code)]

//! End-to-end test for the `connect` and `disconnect` subcommands.
//!
//! Spawns the real `slugaudit` binary as a subprocess against hermetic temp
//! homes and fake client CLIs on a replaced `PATH`.

use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Case {
    agent: &'static str,
    cli: bool,
    invocations: &'static [&'static str],
    written: Option<&'static str>,
}

const CASES: &[Case] = &[
    Case {
        agent: "codex",
        cli: true,
        invocations: &["mcp add slugaudit -- {bin}"],
        written: None,
    },
    Case {
        agent: "claude",
        cli: true,
        invocations: &["mcp add -s user slugaudit -- {bin}"],
        written: None,
    },
    Case {
        agent: "grok",
        cli: true,
        invocations: &["mcp add slugaudit --scope user -- {bin}"],
        written: None,
    },
    Case {
        agent: "gemini",
        cli: true,
        invocations: &["mcp add -s user slugaudit {bin}"],
        written: None,
    },
    Case {
        agent: "bob",
        cli: true,
        invocations: &["mcp add --transport stdio --scope global slugaudit {bin}"],
        written: None,
    },
    Case {
        agent: "droid",
        cli: true,
        invocations: &["mcp add --type stdio slugaudit {bin}"],
        written: None,
    },
    Case {
        agent: "openhands",
        cli: true,
        invocations: &["mcp add slugaudit --transport stdio {bin}"],
        written: None,
    },
    Case {
        agent: "hermes",
        cli: true,
        invocations: &["mcp add slugaudit --command {bin}"],
        written: None,
    },
    Case {
        agent: "q",
        cli: true,
        invocations: &["mcp add --name slugaudit --command {bin}"],
        written: None,
    },
    Case {
        agent: "cursor",
        cli: false,
        invocations: &[],
        written: Some(".cursor/mcp.json"),
    },
    Case {
        agent: "omp",
        cli: false,
        invocations: &[],
        written: Some(".omp/agent/mcp.json"),
    },
];

fn write_fake_cli(bin_dir: &Path, name: &str, log: &Path) -> TestResult {
    let script = bin_dir.join(name);
    let probe_guard = if name == "q" {
        "if [ \"$*\" = \"mcp --help\" ]; then\n  printf 'Usage: q mcp <COMMAND>\\nCommands:\\n  add   Add a server\\n'\n  exit 0\nfi\n"
    } else {
        ""
    };
    fs::write(
        &script,
        format!(
            "#!/bin/sh\n{probe_guard}printf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            log.display()
        ),
    )
    .check("write fake cli")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).check("metadata")?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).check("chmod fake cli")?;
    }
    Ok(())
}

fn hermetic_output(
    binary: &Path,
    home: &Path,
    bin_dir: &Path,
    args: &[&str],
) -> TestResult<std::process::Output> {
    Command::new(binary)
        .args(args)
        .env("HOME", home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", home)
        .env("PATH", bin_dir)
        .output()
        .check("spawn slugaudit")
}

#[test]
fn connect_emits_the_documented_argv_for_every_client_dialect() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    let log = temp.path().join("argv.log");
    fs::create_dir_all(&home).check("mkdir home")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    for case in CASES {
        if case.cli {
            write_fake_cli(&bin_dir, case.agent, &log)?;
        }
    }

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit"));
    ensure(
        binary.is_file(),
        format!("test binary not found: {}", binary.display()),
    )?;

    let install = hermetic_output(&binary, &home, &bin_dir, &["install"])?;
    ensure(
        install.status.success(),
        format!(
            "install failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&install.stdout),
            String::from_utf8_lossy(&install.stderr)
        ),
    )?;
    let installed = home.join(".slugthug/slugaudit/slugaudit");
    ensure(
        installed.is_file(),
        format!("install did not produce {}", installed.display()),
    )?;

    for case in CASES {
        fs::write(&log, "").check("truncate argv log")?;
        let output = hermetic_output(&binary, &home, &bin_dir, &["connect", case.agent])?;
        ensure(
            output.status.success(),
            format!(
                "{} connect failed:\nstdout: {}\nstderr: {}",
                case.agent,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;

        let expected: Vec<String> = case
            .invocations
            .iter()
            .map(|line| line.replace("{bin}", &installed.display().to_string()))
            .collect();
        let recorded: Vec<String> = fs::read_to_string(&log)
            .check("read argv log")?
            .lines()
            .map(str::to_string)
            .collect();
        ensure_eq(
            &recorded,
            &expected,
            format!(
                "{} emitted the wrong argv (install path {})",
                case.agent,
                installed.display()
            ),
        )?;

        if let Some(relative) = case.written {
            let path = home.join(relative);
            let content = fs::read_to_string(&path).check(&format!(
                "{} must write {}",
                case.agent,
                path.display()
            ))?;
            ensure(
                content.contains("slugaudit") && content.contains(&installed.display().to_string()),
                format!(
                    "{} wrote {} without a slugaudit entry pointing at {}:\n{content}",
                    case.agent,
                    path.display(),
                    installed.display()
                ),
            )?;
        }
    }
    Ok(())
}

#[test]
fn disconnecting_an_entry_with_nothing_to_remove_says_so() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&home).check("mkdir home")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit"));

    let output = hermetic_output(&binary, &home, &bin_dir, &["disconnect", "goose"])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure(
        output.status.success(),
        format!(
            "a no-op disconnect is not an error: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    ensure(
        stdout.contains("Nothing to disconnect."),
        format!("a no-op must be reported as one: {stdout}"),
    )?;
    ensure(
        !stdout.contains("Done."),
        format!("nothing was done: {stdout}"),
    )?;
    Ok(())
}

#[test]
fn disconnecting_a_config_without_an_entry_leaves_it_untouched() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(home.join(".cursor")).check("mkdir cursor")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    let config = home.join(".cursor/mcp.json");
    let original = "{\n  \"mcpServers\": {\"other\": {\"command\": \"/bin/other\"}}\n}";
    fs::write(&config, original).check("write config without slugaudit")?;
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit"));

    let output = hermetic_output(&binary, &home, &bin_dir, &["disconnect", "cursor"])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure(
        output.status.success(),
        format!(
            "a no-op disconnect is not an error: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    ensure(
        stdout.contains("Nothing to disconnect."),
        format!("a config without an entry must be reported as nothing to do: {stdout}"),
    )?;
    ensure(
        !stdout.contains("Done."),
        format!("nothing was done: {stdout}"),
    )?;
    ensure_eq(
        &fs::read_to_string(&config).check("read config")?,
        &original,
        "an untouched config must not be rewritten or reformatted",
    )?;
    Ok(())
}

#[test]
fn connect_fails_a_malformed_client_config_and_names_the_json_cause() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(home.join(".cursor")).check("mkdir cursor")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    fs::write(home.join(".cursor/mcp.json"), "{ not json").check("write malformed config")?;

    let output = Command::new(env!("CARGO_BIN_EXE_slugaudit"))
        .args(["connect", "cursor"])
        .env("HOME", &home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &bin_dir)
        .env("RUST_LOG", "info")
        .env_remove("SLUGAUDIT_LOG_FORMAT")
        .output()
        .check("spawn slugaudit")?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    ensure(
        !output.status.success(),
        "a malformed config must fail the connect",
    )?;
    ensure(
        stderr.contains("could not tell whether cursor is connected"),
        format!("the probe must report that it could not tell: {stderr}"),
    )?;
    ensure(
        stderr.contains("contains invalid JSON"),
        format!("the error must name the JSON cause: {stderr}"),
    )?;
    Ok(())
}

#[test]
fn interactive_connect_reports_end_of_input_instead_of_an_invalid_choice() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(home.join(".cursor")).check("mkdir cursor")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;

    let output = Command::new(env!("CARGO_BIN_EXE_slugaudit"))
        .arg("connect")
        .env("HOME", &home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &bin_dir)
        .stdin(Stdio::null())
        .output()
        .check("spawn slugaudit")?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    ensure(
        !output.status.success(),
        "end of input must fail the command",
    )?;
    ensure(stderr.contains("input ended"), format!("stderr: {stderr}"))?;
    ensure(
        !stderr.contains("invalid agent choice"),
        format!("stderr: {stderr}"),
    )?;
    Ok(())
}

/// Interactive `slugaudit disconnect` lists connected JSON/JSONC configs and
/// removes the selected entry while reporting manual-only configs explicitly.
#[test]
fn interactive_disconnect_removes_chosen_opencode_jsonc_and_notes_manual_goose() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(home.join(".config/opencode")).check("mkdir opencode")?;
    fs::create_dir_all(home.join(".config/goose")).check("mkdir goose")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;

    let opencode_cfg = home.join(".config/opencode/opencode.jsonc");
    fs::write(
        &opencode_cfg,
        "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {\n    \"slugaudit\": {\n      \"type\": \"local\",\n      \"command\": [\"/bin/slugaudit\",],\n      \"enabled\": true,\n    },\n  },\n}\n",
    )
    .check("write opencode.jsonc")?;

    let goose_cfg = home.join(".config/goose/config.yaml");
    fs::write(
        &goose_cfg,
        "extensions:\n  slugaudit:\n    type: stdio\n    cmd: /bin/slugaudit\n",
    )
    .check("write goose config")?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_slugaudit"))
        .arg("disconnect")
        .env("HOME", &home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &bin_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .check("spawn slugaudit disconnect")?;

    {
        let mut stdin = child.stdin.take().check("take stdin")?;
        writeln!(stdin, "1").check("select option 1")?;
    }
    let output = child.wait_with_output().check("wait for disconnect")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure(
        output.status.success(),
        format!(
            "interactive disconnect failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    ensure(
        stdout.contains("Goose CLI is configured manually"),
        format!("manual goose removal note expected: {stdout}"),
    )?;
    ensure(
        stdout.contains("1) OpenCode") && stdout.contains("Done."),
        format!("OpenCode must be selectable and removed: {stdout}"),
    )?;
    ensure(
        !fs::read_to_string(&opencode_cfg)
            .check("read opencode.jsonc")?
            .contains("slugaudit"),
        "slugaudit entry removed from opencode.jsonc",
    )?;
    Ok(())
}

#[test]
fn connect_and_disconnect_refuse_to_destroy_comments_in_jsonc_config() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(home.join(".config/opencode")).check("mkdir opencode")?;
    fs::create_dir_all(&bin_dir).check("mkdir bin")?;
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit"));

    let opencode_cfg = home.join(".config/opencode/opencode.jsonc");
    let original = "{\n  // Keep user comments intact\n  \"mcp\": {\n    \"slugaudit\": {\n      \"type\": \"local\",\n      \"command\": [\"/bin/slugaudit\"]\n    }\n  }\n}\n";
    fs::write(&opencode_cfg, original).check("write commented opencode.jsonc")?;

    let disconnect_out = hermetic_output(&binary, &home, &bin_dir, &["disconnect", "opencode"])?;
    ensure(
        !disconnect_out.status.success(),
        "disconnect must refuse to erase comments in opencode.jsonc",
    )?;
    ensure(
        String::from_utf8_lossy(&disconnect_out.stderr).contains("slugaudit"),
        "error message names the slugaudit entry to edit manually",
    )?;
    ensure_eq(
        &fs::read_to_string(&opencode_cfg).check("read preserved opencode.jsonc")?,
        &original,
        "commented opencode.jsonc must remain byte-for-byte untouched",
    )?;
    Ok(())
}
