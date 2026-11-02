use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn no_arguments_means_serve() -> TestResult {
    assert_eq!(
        parse_args(std::iter::empty()).check("valid arguments parse")?,
        Command::Serve
    );
    Ok(())
}

#[test]
fn usage_with_a_plain_style_is_byte_identical_to_the_usage_text() {
    let rendered = render_usage(&Style::plain());

    assert_eq!(
        rendered,
        usage_text(),
        "plain help must match the usage text"
    );
}

#[test]
fn usage_with_a_colored_style_keeps_every_heading_and_styles_it() {
    let rendered = render_usage(&Style::colored());

    for heading in ["USAGE:", "COMMANDS:", "OPTIONS:", "EXAMPLES:"] {
        assert!(rendered.contains(heading), "help must keep {heading:?}");
    }
    assert!(
        rendered.contains("\x1b[1m\x1b[36mCOMMANDS:\x1b[0m\x1b[0m"),
        "headings are bold cyan: {rendered:?}"
    );
}

#[test]
fn explicit_serve() -> TestResult {
    assert_eq!(
        parse_args(vec!["serve".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Serve
    );
    Ok(())
}

#[test]
fn unrecognized_input_returns_a_descriptive_error_rather_than_silently_serving() -> TestResult {
    let err = parse_args(vec!["foobar".to_owned()].into_iter()).check_err("the call must fail")?;
    assert!(err.contains("unknown command"));
    assert!(err.contains("foobar"));
    assert!(err.contains("serve") && err.contains("connect") && err.contains("install"));
    assert!(err.contains("menu") && err.contains("disconnect"));
    Ok(())
}

#[test]
fn explicit_help_command_parses_as_help() -> TestResult {
    assert_eq!(
        parse_args(vec!["help".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Help
    );
    Ok(())
}

#[test]
fn case_mismatched_known_command_is_an_error() -> TestResult {
    let result = parse_args(vec!["SErve".to_owned()].into_iter());
    let err = result.check_err("case mismatched command should fail")?;
    assert!(err.contains("unknown command"));
    assert!(err.contains("SErve"));
    Ok(())
}

#[test]
fn connect_with_no_agent_picks_interactive() -> TestResult {
    assert_eq!(
        parse_args(vec!["connect".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Connect { agent: None }
    );
    Ok(())
}

#[test]
fn connect_accepts_agent_name() -> TestResult {
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "claude".to_owned()].into_iter())
            .check("valid arguments parse")?,
        Command::Connect {
            agent: Some("claude".to_string())
        }
    );
    Ok(())
}

#[test]
fn disconnect_with_no_agent_picks_interactive() -> TestResult {
    assert_eq!(
        parse_args(vec!["disconnect".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Disconnect { agent: None }
    );
    Ok(())
}

#[test]
fn disconnect_accepts_agent_name() -> TestResult {
    assert_eq!(
        parse_args(vec!["disconnect".to_owned(), "agy".to_owned()].into_iter())
            .check("valid arguments parse")?,
        Command::Disconnect {
            agent: Some("agy".to_string())
        }
    );
    Ok(())
}

#[test]
fn remove_alias_parses_as_disconnect() -> TestResult {
    assert_eq!(
        parse_args(vec!["remove".to_owned(), "agy".to_owned()].into_iter())
            .check("valid arguments parse")?,
        Command::Disconnect {
            agent: Some("agy".to_string())
        }
    );
    Ok(())
}

#[test]
fn install_parses_as_its_own_command() -> TestResult {
    assert_eq!(
        parse_args(vec!["install".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Install
    );
    Ok(())
}

#[test]
fn parse_install_without_args_ok() -> TestResult {
    assert_eq!(
        parse_args(vec!["install".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Install
    );
    Ok(())
}

#[test]
fn parse_install_rejects_extra_args() -> TestResult {
    let err = parse_args(vec!["install".to_owned(), "--something".to_owned()].into_iter())
        .check_err("the call must fail")?;
    assert_eq!(
        err,
        "`install` takes no arguments; unexpected argument \"--something\""
    );
    Ok(())
}

#[test]
fn menu_parses_as_its_own_command() -> TestResult {
    assert_eq!(
        parse_args(vec!["menu".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Menu
    );
    Ok(())
}

#[test]
fn parse_menu_without_args_ok() -> TestResult {
    assert_eq!(
        parse_args(vec!["menu".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Menu
    );
    Ok(())
}

#[test]
fn parse_menu_rejects_extra_args() -> TestResult {
    let err = parse_args(vec!["menu".to_owned(), "--wat".to_owned()].into_iter())
        .check_err("the call must fail")?;
    assert_eq!(
        err,
        "`menu` takes no arguments; unexpected argument \"--wat\""
    );
    Ok(())
}

#[test]
fn unrecognized_input_lists_menu_among_the_known_commands() -> TestResult {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).check_err("the call must fail")?;
    assert!(err.contains("menu"), "the error should name menu: {err}");
    Ok(())
}

#[test]
fn update_parses_as_its_own_command() -> TestResult {
    assert_eq!(
        parse_args(vec!["update".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Update
    );
    Ok(())
}

#[test]
fn unrecognized_input_lists_update_among_the_known_commands() -> TestResult {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).check_err("the call must fail")?;
    assert!(
        err.contains("update"),
        "the error should name update: {err}"
    );
    Ok(())
}

#[test]
fn version_parses_in_all_three_spellings() -> TestResult {
    for arg in ["version", "--version", "-V"] {
        assert_eq!(
            parse_args(vec![arg.to_owned()].into_iter()).check("valid arguments parse")?,
            Command::Version,
            "{arg} should parse as Version"
        );
    }
    Ok(())
}

#[test]
fn help_names_the_installed_binary_connect_registers() {
    assert!(
        usage_text().contains("Register the installed SlugAudit binary"),
        "connect prefers the installed binary, so the help must say so"
    );
    assert!(
        !usage_text().contains("Register this binary"),
        "the running binary is only used when no installed copy exists"
    );
}

#[test]
fn a_serde_failure_converts_to_the_json_error_variant() -> TestResult {
    let source = serde_json::from_str::<serde_json::Value>("{").check_err("invalid JSON")?;

    let error = ConnectError::from(source);

    match error {
        ConnectError::Json(inner) => {
            assert!(inner.is_eof(), "the parse error must be kept: {inner}");
        }
        other => return Err(format!("expected Json, got {other:?}")).check("unexpected outcome"),
    }
    Ok(())
}

#[test]
fn a_config_write_without_a_home_reports_the_home_unavailable() -> TestResult {
    let cursor = crate::connect_agents::find_agent("cursor").check("cursor is registered")?;

    let error =
        crate::connect_exec::connect_agent(&cursor, std::path::Path::new("/bin/slugaudit"), None)
            .check_err("a config write has nowhere to go without a home")?;

    assert!(
        matches!(error, ConnectError::HomeUnavailable),
        "expected HomeUnavailable, got {error:?}"
    );
    Ok(())
}

#[test]
fn a_config_parent_that_cannot_be_created_is_an_io_error() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    std::fs::write(temp.path().join(".cursor"), b"not a directory").check("blocking file")?;
    let cursor = crate::connect_agents::find_agent("cursor").check("cursor is registered")?;

    let error = crate::connect_exec::connect_agent(
        &cursor,
        std::path::Path::new("/bin/slugaudit"),
        Some(temp.path()),
    )
    .check_err("create_dir_all must fail on a file")?;

    assert!(
        matches!(error, ConnectError::Io(_)),
        "expected Io, got {error:?}"
    );
    Ok(())
}

#[test]
fn a_config_path_that_is_a_directory_is_an_io_error() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;
    std::fs::create_dir_all(temp.path().join(".cursor/mcp.json")).check("config as directory")?;
    let cursor = crate::connect_agents::find_agent("cursor").check("cursor is registered")?;

    let error = crate::connect_exec::connect_agent(
        &cursor,
        std::path::Path::new("/bin/slugaudit"),
        Some(temp.path()),
    )
    .check_err("reading a directory as a config must fail")?;

    assert!(
        matches!(error, ConnectError::Io(_)),
        "expected Io, got {error:?}"
    );
    Ok(())
}

/// A writer that fails every write with `kind`; the CLI's output helper must
/// return that failure rather than panic.
struct FailingWriter(std::io::ErrorKind);

impl std::io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(self.0, "injected write failure"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_closed_pipe_write_is_reported_as_broken_pipe_not_a_panic() -> TestResult {
    let mut writer = FailingWriter(std::io::ErrorKind::BrokenPipe);

    let error = write_to(&mut writer, format_args!("partial output"))
        .check_err("a closed pipe must surface as an error")?;

    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(is_broken_pipe(&error));
    Ok(())
}

#[test]
fn a_non_pipe_write_failure_is_reported_and_not_classified_as_a_clean_exit() -> TestResult {
    let mut writer = FailingWriter(std::io::ErrorKind::StorageFull);

    let error = write_to(&mut writer, format_args!("partial output"))
        .check_err("a write failure must surface as an error")?;

    assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(error.to_string(), "injected write failure");
    assert!(!is_broken_pipe(&error));
    Ok(())
}

#[test]
fn a_successful_write_keeps_the_exact_output_text() -> TestResult {
    let mut writer = Vec::new();

    write_to(&mut writer, format_args!("slugaudit {}\n", "1.0.10")).check("the write succeeds")?;

    assert_eq!(writer, b"slugaudit 1.0.10\n");
    Ok(())
}

#[test]
fn a_closed_pipe_is_found_through_every_cli_error_wrapper() {
    let broken = || std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed pipe");

    assert!(is_broken_pipe(&ConnectError::Io(broken())));
    assert!(is_broken_pipe(&crate::install::InstallError::Io(broken())));
    assert!(is_broken_pipe(&crate::update::UpdateError::Io(broken())));
}

#[test]
fn a_non_pipe_io_failure_in_a_wrapper_is_not_a_clean_exit() {
    let refused = || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");

    assert!(!is_broken_pipe(&ConnectError::Io(refused())));
    assert!(!is_broken_pipe(
        &crate::install::InstallError::Io(refused())
    ));
    assert!(!is_broken_pipe(&crate::update::UpdateError::Io(refused())));
}

#[test]
fn a_rejected_add_with_output_keeps_the_pointer_to_it() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("tempdir")?;
    let bin_dir = tempfile::tempdir().check("bin dir")?;
    let script = bin_dir.path().join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\necho 'client rejected it' >&2\nexit 1\n",
    )
    .check("write fake cli")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script).check("metadata")?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).check("chmod fake cli")?;
    }

    let result = temp_env::with_vars(
        [
            ("PATH", Some(bin_dir.path().as_os_str())),
            ("SLUGTHUG_CONFIG_HOME", Some(temp.path().as_os_str())),
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
        ],
        || crate::connect::run_connect(Some("claude")),
    );

    let error = result.check_err("expected AddFailed, got Ok")?;
    match &error {
        ConnectError::AddFailed { cli, .. } => {
            assert_eq!(cli, "claude");
            assert!(
                error.to_string().contains("see the output above"),
                "captured client output makes the pointer true: {error}"
            );
        }
        other => {
            return Err(format!("expected AddFailed, got {other:?}")).check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn cli_exit_lowercase_v_parses_as_version() -> TestResult {
    assert_eq!(
        parse_args(vec!["-v".to_owned()].into_iter()).check("valid arguments parse")?,
        Command::Version
    );
    Ok(())
}

#[test]
fn cli_exit_connect_with_an_extra_argument_is_a_usage_error() -> TestResult {
    let err = parse_args(vec!["connect".to_owned(), "a".to_owned(), "b".to_owned()].into_iter())
        .check_err("the call must fail")?;
    assert_eq!(
        err,
        "`connect` takes at most one agent name; unexpected argument \"b\""
    );
    Ok(())
}

#[test]
fn cli_exit_disconnect_and_remove_with_an_extra_argument_are_usage_errors() -> TestResult {
    for word in ["disconnect", "remove"] {
        let err = parse_args(vec![word.to_owned(), "a".to_owned(), "b".to_owned()].into_iter())
            .check_err("the call must fail")?;
        assert!(
            err.starts_with(&format!("`{word}` takes at most one agent name")),
            "{err}"
        );
    }
    Ok(())
}

fn parse(args: &[&str]) -> Result<Command, String> {
    parse_args(args.iter().map(|arg| (*arg).to_owned()))
}

#[test]
fn every_subcommand_help_flag_in_any_position_parses_to_that_subcommands_help() -> TestResult {
    let cases = [
        ("serve", Topic::Serve),
        ("connect", Topic::Connect),
        ("disconnect", Topic::Disconnect),
        ("remove", Topic::Disconnect),
        ("install", Topic::Install),
        ("menu", Topic::Menu),
        ("update", Topic::Update),
    ];
    for (name, topic) in cases {
        for flag in ["--help", "-h"] {
            for args in [
                vec![name, flag],
                vec![name, "x", flag],
                vec![name, flag, "x"],
            ] {
                assert_eq!(
                    parse(&args).check("help parses")?,
                    Command::CommandHelp(topic),
                    "{args:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn update_help_never_parses_to_the_update_command() -> TestResult {
    for args in [["update", "--help"], ["update", "-h"]] {
        assert_ne!(parse(&args).check("help parses")?, Command::Update);
    }
    Ok(())
}

#[test]
fn every_subcommand_has_its_own_usage_naming_that_subcommand() {
    for (topic, needle) in [
        (Topic::Serve, "slugaudit serve"),
        (Topic::Connect, "slugaudit connect [AGENT]"),
        (Topic::Disconnect, "slugaudit disconnect [AGENT]"),
        (Topic::Install, "slugaudit install"),
        (Topic::Menu, "slugaudit menu"),
        (Topic::Update, "slugaudit update"),
    ] {
        let text = topic_usage(topic);
        assert!(text.starts_with("USAGE:\n"), "{text}");
        assert!(text.contains(needle), "{needle}: {text}");
        assert!(text.contains("-h, --help"), "{text}");
    }
}

#[test]
fn general_help_lists_serve_and_shares_every_description_with_the_subcommand_help() {
    let general = usage_text();
    assert!(
        general.contains(
            "    slugaudit serve                  Run the MCP server (stdio transport)\n"
        ),
        "{general}"
    );
    for topic in [
        Topic::Serve,
        Topic::Connect,
        Topic::Disconnect,
        Topic::Install,
        Topic::Menu,
        Topic::Update,
    ] {
        let own = topic_usage(topic);
        let description = own
            .lines()
            .filter(|line| {
                !line.is_empty() && !line.starts_with("USAGE:") && !line.starts_with(' ')
            })
            .filter(|line| *line != "OPTIONS:");
        for line in description {
            assert!(general.contains(line), "general help lacks {line:?}");
        }
    }
}
