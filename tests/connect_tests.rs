#![forbid(unsafe_code)]

//! End-to-end test for the `connect` subcommand.
//!
//! This spawns the real `slugaudit` binary as a subprocess and runs
//! `connect <agent>` against a fake client CLI, then asserts the exact argv
//! slugaudit emitted. Nothing outside a temporary directory is read or
//! written: `HOME`, `SLUGTHUG_HOME` and `SLUGTHUG_CONFIG_HOME` all point into
//! the temp dir, and `PATH` is *replaced* (not prepended to) by a directory of
//! fake clients — one shell script per agent name that appends its own argv,
//! one line per invocation, to a log file and exits 0. The installed binary is
//! the one `install` just copied into the temp home, so the asserted argument
//! is a temp path too.
//!
//! This replaced an earlier version that drove the *real* client binaries
//! (`bob`, `claude`, `grok`, `codex`) against the real home, backing up and
//! restoring the user's live configs and self-skipping when no client CLI was
//! installed. That version was neither hermetic nor a test of slugaudit: what
//! it verified depended on which clients happened to be on the machine, and
//! `is_connected` executing the installed `codex` once rewrote the owner's
//! `~/.codex/config.toml` during a run (`.planning/DEVIATIONS.md`). The fake
//! CLI removes both problems — it runs deterministically everywhere, with no
//! skip, and it asserts the registration argv itself rather than a client's
//! interpretation of it.
//!
//! The `install` step deliberately runs into the temp `SLUGTHUG_HOME`, never
//! `~/.slugthug`, so it leaves the machine exactly as it found it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One client whose registration is exercised end to end.
struct Case {
    /// The agent id passed to `connect` (also the fake CLI's file name, when
    /// the agent is CLI-driven).
    agent: &'static str,
    /// Whether a fake CLI is put on `PATH` for this agent.
    cli: bool,
    /// The invocations the fake CLI must record, in order. Each entry is one
    /// invocation with its argv space-joined; `{bin}` stands for the installed
    /// slugaudit binary and is substituted at run time.
    invocations: &'static [&'static str],
    /// Home-relative config file `connect` must write for a client that has no
    /// add command.
    written: Option<&'static str>,
}

/// The dialects the registry actually emits. `codex` is the plain `--`
/// separator, `claude`/`gemini` the `-s user` scope forms (with and without
/// the separator), `grok` a `--scope user` positional, `bob` transport *and*
/// scope, `droid` `--type`, `openhands` `--transport`, `hermes` and `q` the
/// `--command` positional spellings. The two `written` rows are the clients
/// with no `mcp add` at all.
///
/// Every CLI-driven case starts with `mcp list`: `connect` asks the client
/// whether slugaudit is already registered before it adds it.
const CASES: &[Case] = &[
    Case {
        agent: "codex",
        cli: true,
        invocations: &["mcp list", "mcp add slugaudit -- {bin}"],
        written: None,
    },
    Case {
        agent: "claude",
        cli: true,
        invocations: &["mcp list", "mcp add -s user slugaudit -- {bin}"],
        written: None,
    },
    Case {
        agent: "grok",
        cli: true,
        invocations: &["mcp list", "mcp add slugaudit --scope user -- {bin}"],
        written: None,
    },
    // Gemini's `--scope` defaults to project, so a global registration passes
    // `-s user` and no `--` separator (probed against gemini 0.64.0).
    Case {
        agent: "gemini",
        cli: true,
        invocations: &["mcp list", "mcp add -s user slugaudit {bin}"],
        written: None,
    },
    Case {
        agent: "bob",
        cli: true,
        invocations: &[
            "mcp list",
            "mcp add --transport stdio --scope global slugaudit {bin}",
        ],
        written: None,
    },
    Case {
        agent: "droid",
        cli: true,
        invocations: &["mcp list", "mcp add --type stdio slugaudit {bin}"],
        written: None,
    },
    Case {
        agent: "openhands",
        cli: true,
        invocations: &["mcp list", "mcp add slugaudit --transport stdio {bin}"],
        written: None,
    },
    Case {
        agent: "hermes",
        cli: true,
        invocations: &["mcp list", "mcp add slugaudit --command {bin}"],
        written: None,
    },
    Case {
        agent: "q",
        cli: true,
        invocations: &["mcp list", "mcp add --name slugaudit --command {bin}"],
        written: None,
    },
    // cursor's only executable is `agent`, a verify-only CLI that cannot
    // register; omp has no CLI at all. Both must fall through to the file.
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

/// Writes a fake client CLI named `name` into `bin_dir`: a shell script that
/// records its argv — one line per invocation, arguments space-separated — to
/// `log` and exits 0. Nothing is printed, so the `mcp list` probe answers "not
/// registered" and `connect` always takes the add path.
fn write_fake_cli(bin_dir: &Path, name: &str, log: &Path) {
    let script = bin_dir.join(name);
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            log.display()
        ),
    )
    .expect("write fake cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod fake cli");
    }
}

/// Spawns the slugaudit binary with every home pointed at `home` and `PATH`
/// reduced to `bin_dir`, so the child cannot reach a real client CLI or a real
/// client config.
fn hermetic_output(
    binary: &Path,
    home: &Path,
    bin_dir: &Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(binary)
        .args(args)
        .env("HOME", home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", home)
        .env("PATH", bin_dir)
        .output()
        .expect("spawn slugaudit")
}

#[test]
fn connect_emits_the_documented_argv_for_every_client_dialect() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    let log = temp.path().join("argv.log");
    fs::create_dir_all(&home).expect("mkdir home");
    fs::create_dir_all(&bin_dir).expect("mkdir bin");
    for case in CASES {
        if case.cli {
            write_fake_cli(&bin_dir, case.agent, &log);
        }
    }

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit"));
    assert!(
        binary.is_file(),
        "test binary not found: {}",
        binary.display()
    );

    // Run `install` so the binary lands at the stable home path `connect`
    // prefers, mirroring the real flow: install once, then connect.
    let install = hermetic_output(&binary, &home, &bin_dir, &["install"]);
    assert!(
        install.status.success(),
        "install failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr),
    );
    let installed = home.join(".slugthug/slugaudit/slugaudit");
    assert!(
        installed.is_file(),
        "install did not produce {}",
        installed.display()
    );

    for case in CASES {
        fs::write(&log, "").expect("truncate argv log");
        let output = hermetic_output(&binary, &home, &bin_dir, &["connect", case.agent]);
        assert!(
            output.status.success(),
            "{} connect failed:\nstdout: {}\nstderr: {}",
            case.agent,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let expected: Vec<String> = case
            .invocations
            .iter()
            .map(|line| line.replace("{bin}", &installed.display().to_string()))
            .collect();
        let recorded: Vec<String> = fs::read_to_string(&log)
            .expect("read argv log")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            recorded,
            expected,
            "{} emitted the wrong argv (install path {})",
            case.agent,
            installed.display()
        );

        if let Some(relative) = case.written {
            let path = home.join(relative);
            let content = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("{} must write {}: {error}", case.agent, path.display())
            });
            assert!(
                content.contains("slugaudit") && content.contains(&installed.display().to_string()),
                "{} wrote {} without a slugaudit entry pointing at {}:\n{content}",
                case.agent,
                path.display(),
                installed.display(),
            );
        }
    }
}

/// A disconnect that removes nothing must not report "Done.": goose has no
/// writable section and no client CLI, so there is nothing this tool can take
/// out.
#[test]
fn disconnecting_an_entry_with_nothing_to_remove_says_so() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&home).expect("mkdir home");
    fs::create_dir_all(&bin_dir).expect("mkdir bin");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_slugaudit"));

    let output = hermetic_output(&binary, &home, &bin_dir, &["disconnect", "goose"]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "a no-op disconnect is not an error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Nothing to disconnect."),
        "a no-op must be reported as one: {stdout}"
    );
    assert!(!stdout.contains("Done."), "nothing was done: {stdout}");
}

/// A warning raised by a CLI command must reach the subscriber installed
/// before dispatch; `connect` probes the client config and logs why an
/// unparsable one is treated as unregistered.
#[test]
fn connect_logs_a_client_config_parse_warning_to_stderr() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(home.join(".cursor")).expect("mkdir cursor");
    fs::create_dir_all(&bin_dir).expect("mkdir bin");
    fs::write(home.join(".cursor/mcp.json"), "{ not json").expect("write malformed config");

    let output = Command::new(env!("CARGO_BIN_EXE_slugaudit"))
        .args(["connect", "cursor"])
        .env("HOME", &home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &bin_dir)
        .env("RUST_LOG", "info")
        .env_remove("SLUGAUDIT_LOG_FORMAT")
        .output()
        .expect("spawn slugaudit");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a malformed config must fail the connect"
    );
    assert!(
        stderr.contains("client config is not valid JSON"),
        "the warning must reach stderr: {stderr}"
    );
}

/// A closed stdin during the interactive agent choice is "input ended", not an
/// empty answer that surfaces as "invalid agent choice".
#[test]
fn interactive_connect_reports_end_of_input_instead_of_an_invalid_choice() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let bin_dir = temp.path().join("bin");
    // A `.cursor` directory is enough for Cursor to be detected as installed.
    fs::create_dir_all(home.join(".cursor")).expect("mkdir cursor");
    fs::create_dir_all(&bin_dir).expect("mkdir bin");

    let output = Command::new(env!("CARGO_BIN_EXE_slugaudit"))
        .arg("connect")
        .env("HOME", &home)
        .env("SLUGTHUG_HOME", home.join(".slugthug"))
        .env("SLUGTHUG_CONFIG_HOME", &home)
        .env("PATH", &bin_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn slugaudit");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "end of input must fail the command"
    );
    assert!(stderr.contains("input ended"), "stderr: {stderr}");
    assert!(!stderr.contains("invalid agent choice"), "stderr: {stderr}");
}
