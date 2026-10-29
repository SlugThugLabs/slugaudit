use super::*;

#[test]
fn no_arguments_means_serve() {
    assert_eq!(
        parse_args(std::iter::empty()).expect("valid arguments parse"),
        Command::Serve
    );
}

#[test]
fn usage_with_a_plain_style_is_byte_identical_to_the_usage_const() {
    let rendered = render_usage(&Style::plain());

    assert_eq!(rendered, USAGE, "plain help must match USAGE exactly");
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
fn explicit_serve() {
    assert_eq!(
        parse_args(vec!["serve".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Serve
    );
}

#[test]
fn unrecognized_input_returns_a_descriptive_error_rather_than_silently_serving() {
    let err = parse_args(vec!["foobar".to_owned()].into_iter()).unwrap_err();
    assert!(err.contains("unknown command"));
    assert!(err.contains("foobar"));
    assert!(err.contains("serve") && err.contains("connect") && err.contains("install"));
    assert!(err.contains("menu") && err.contains("disconnect"));
}

#[test]
fn explicit_help_command_parses_as_help() {
    assert_eq!(
        parse_args(vec!["help".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Help
    );
}

#[test]
fn case_mismatched_known_command_is_an_error() {
    let result = parse_args(vec!["SErve".to_owned()].into_iter());
    let err = result.expect_err("case mismatched command should fail");
    assert!(err.contains("unknown command"));
    assert!(err.contains("SErve"));
}

#[test]
fn connect_with_no_agent_picks_interactive() {
    assert_eq!(
        parse_args(vec!["connect".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Connect { agent: None }
    );
}

#[test]
fn connect_accepts_agent_name() {
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "claude".to_owned()].into_iter())
            .expect("valid arguments parse"),
        Command::Connect {
            agent: Some("claude".to_string())
        }
    );
}

#[test]
fn disconnect_with_no_agent_picks_interactive() {
    assert_eq!(
        parse_args(vec!["disconnect".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Disconnect { agent: None }
    );
}

#[test]
fn disconnect_accepts_agent_name() {
    assert_eq!(
        parse_args(vec!["disconnect".to_owned(), "agy".to_owned()].into_iter())
            .expect("valid arguments parse"),
        Command::Disconnect {
            agent: Some("agy".to_string())
        }
    );
}

#[test]
fn remove_alias_parses_as_disconnect() {
    assert_eq!(
        parse_args(vec!["remove".to_owned(), "agy".to_owned()].into_iter())
            .expect("valid arguments parse"),
        Command::Disconnect {
            agent: Some("agy".to_string())
        }
    );
}

#[test]
fn install_parses_as_its_own_command() {
    assert_eq!(
        parse_args(vec!["install".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Install
    );
}

#[test]
fn install_ignores_any_extra_arguments() {
    assert_eq!(
        parse_args(vec!["install".to_owned(), "--something".to_owned()].into_iter())
            .expect("valid arguments parse"),
        Command::Install
    );
}

#[test]
fn menu_parses_as_its_own_command() {
    assert_eq!(
        parse_args(vec!["menu".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Menu
    );
}

#[test]
fn menu_ignores_any_extra_arguments() {
    assert_eq!(
        parse_args(vec!["menu".to_owned(), "--wat".to_owned()].into_iter())
            .expect("valid arguments parse"),
        Command::Menu
    );
}

#[test]
fn unrecognized_input_lists_menu_among_the_known_commands() {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).unwrap_err();
    assert!(err.contains("menu"), "the error should name menu: {err}");
}

#[test]
fn update_parses_as_its_own_command() {
    assert_eq!(
        parse_args(vec!["update".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Update
    );
}

#[test]
fn unrecognized_input_lists_update_among_the_known_commands() {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).unwrap_err();
    assert!(
        err.contains("update"),
        "the error should name update: {err}"
    );
}

#[test]
fn version_parses_in_all_three_spellings() {
    for arg in ["version", "--version", "-V"] {
        assert_eq!(
            parse_args(vec![arg.to_owned()].into_iter()).expect("valid arguments parse"),
            Command::Version,
            "{arg} should parse as Version"
        );
    }
}

#[test]
fn help_names_the_installed_binary_connect_registers() {
    assert!(
        USAGE.contains("Register the installed SlugAudit binary"),
        "connect prefers the installed binary, so the help must say so"
    );
    assert!(
        !USAGE.contains("Register this binary"),
        "the running binary is only used when no installed copy exists"
    );
}

#[test]
fn a_serde_failure_converts_to_the_json_error_variant() {
    let source = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON");

    let error = ConnectError::from(source);

    match error {
        ConnectError::Json(inner) => {
            assert!(inner.is_eof(), "the parse error must be kept: {inner}");
        }
        other => panic!("expected Json, got {other:?}"),
    }
}

#[test]
fn a_config_write_without_a_home_reports_the_home_unavailable() {
    let cursor = crate::connect_agents::find_agent("cursor").expect("cursor is registered");

    let error =
        crate::connect_exec::connect_agent(&cursor, std::path::Path::new("/bin/slugaudit"), None)
            .expect_err("a config write has nowhere to go without a home");

    assert!(
        matches!(error, ConnectError::HomeUnavailable),
        "expected HomeUnavailable, got {error:?}"
    );
}

#[test]
fn a_config_parent_that_cannot_be_created_is_an_io_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::write(temp.path().join(".cursor"), b"not a directory").expect("blocking file");
    let cursor = crate::connect_agents::find_agent("cursor").expect("cursor is registered");

    let error = crate::connect_exec::connect_agent(
        &cursor,
        std::path::Path::new("/bin/slugaudit"),
        Some(temp.path()),
    )
    .expect_err("create_dir_all must fail on a file");

    assert!(
        matches!(error, ConnectError::Io(_)),
        "expected Io, got {error:?}"
    );
}

#[test]
fn a_config_path_that_is_a_directory_is_an_io_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(temp.path().join(".cursor/mcp.json")).expect("config as directory");
    let cursor = crate::connect_agents::find_agent("cursor").expect("cursor is registered");

    let error = crate::connect_exec::connect_agent(
        &cursor,
        std::path::Path::new("/bin/slugaudit"),
        Some(temp.path()),
    )
    .expect_err("reading a directory as a config must fail");

    assert!(
        matches!(error, ConnectError::Io(_)),
        "expected Io, got {error:?}"
    );
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
fn a_closed_pipe_write_is_reported_as_broken_pipe_not_a_panic() {
    let mut writer = FailingWriter(std::io::ErrorKind::BrokenPipe);

    let error = write_to(&mut writer, format_args!("partial output"))
        .expect_err("a closed pipe must surface as an error");

    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(is_broken_pipe(&error));
}

#[test]
fn a_non_pipe_write_failure_is_reported_and_not_classified_as_a_clean_exit() {
    let mut writer = FailingWriter(std::io::ErrorKind::StorageFull);

    let error = write_to(&mut writer, format_args!("partial output"))
        .expect_err("a write failure must surface as an error");

    assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(error.to_string(), "injected write failure");
    assert!(!is_broken_pipe(&error));
}

#[test]
fn a_successful_write_keeps_the_exact_output_text() {
    let mut writer = Vec::new();

    write_to(&mut writer, format_args!("slugaudit {}\n", "1.0.10")).expect("the write succeeds");

    assert_eq!(writer, b"slugaudit 1.0.10\n");
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
fn a_rejected_add_with_output_keeps_the_pointer_to_it() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = tempfile::tempdir().expect("bin dir");
    let script = bin_dir.path().join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\necho 'client rejected it' >&2\nexit 1\n",
    )
    .expect("write fake cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod fake cli");
    }

    let result = temp_env::with_vars(
        [
            ("PATH", Some(bin_dir.path().as_os_str())),
            ("SLUGTHUG_CONFIG_HOME", Some(temp.path().as_os_str())),
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
        ],
        || crate::connect::run_connect(Some("claude")),
    );

    let error = result.expect_err("expected AddFailed, got Ok");
    match &error {
        ConnectError::AddFailed { cli, .. } => {
            assert_eq!(cli, "claude");
            assert!(
                error.to_string().contains("see the output above"),
                "captured client output makes the pointer true: {error}"
            );
        }
        other => panic!("expected AddFailed, got {other:?}"),
    }
}

#[test]
fn cli_exit_lowercase_v_parses_as_version() {
    assert_eq!(
        parse_args(vec!["-v".to_owned()].into_iter()).expect("valid arguments parse"),
        Command::Version
    );
}

#[test]
fn cli_exit_connect_with_an_extra_argument_is_a_usage_error() {
    let err = parse_args(vec!["connect".to_owned(), "a".to_owned(), "b".to_owned()].into_iter())
        .unwrap_err();
    assert_eq!(
        err,
        "`connect` takes at most one agent name; unexpected argument \"b\""
    );
}

#[test]
fn cli_exit_disconnect_and_remove_with_an_extra_argument_are_usage_errors() {
    for word in ["disconnect", "remove"] {
        let err = parse_args(vec![word.to_owned(), "a".to_owned(), "b".to_owned()].into_iter())
            .unwrap_err();
        assert!(
            err.starts_with(&format!("`{word}` takes at most one agent name")),
            "{err}"
        );
    }
}
