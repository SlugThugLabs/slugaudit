use super::*;

#[test]
fn no_arguments_means_serve() {
    assert_eq!(parse_args(std::iter::empty()).unwrap(), Command::Serve);
}

#[test]
fn explicit_serve() {
    assert_eq!(
        parse_args(vec!["serve".to_owned()].into_iter()).unwrap(),
        Command::Serve
    );
}

#[test]
fn enable_defaults_to_current_directory() {
    assert_eq!(
        parse_args(vec!["enable".to_owned()].into_iter()).unwrap(),
        Command::Enable {
            path: PathBuf::from(".")
        }
    );
}

#[test]
fn enable_with_an_explicit_path() {
    assert_eq!(
        parse_args(vec!["enable".to_owned(), "some/project".to_owned()].into_iter()).unwrap(),
        Command::Enable {
            path: PathBuf::from("some/project")
        }
    );
}

#[test]
fn disable_parses_the_yes_flag_in_either_form() {
    assert_eq!(
        parse_args(vec!["disable".to_owned(), "-y".to_owned()].into_iter()).unwrap(),
        Command::Disable {
            path: PathBuf::from("."),
            assume_yes: true
        }
    );
    assert_eq!(
        parse_args(vec!["disable".to_owned(), "a/b".to_owned(), "--yes".to_owned()].into_iter())
            .unwrap(),
        Command::Disable {
            path: PathBuf::from("a/b"),
            assume_yes: true
        }
    );
}

#[test]
fn unrecognized_input_shows_help_rather_than_silently_serving() {
    assert_eq!(
        parse_args(vec!["--bogus".to_owned()].into_iter()).unwrap(),
        Command::Help
    );
    assert_eq!(
        parse_args(vec!["help".to_owned()].into_iter()).unwrap(),
        Command::Help
    );
}

// --- connect ---

#[test]
fn connect_with_no_agent_picks_interactive() {
    assert_eq!(
        parse_args(vec!["connect".to_owned()].into_iter()).unwrap(),
        Command::Connect { agent: None }
    );
}

#[test]
fn connect_accepts_each_supported_agent_by_cli_name() {
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "claude".to_owned()].into_iter()).unwrap(),
        Command::Connect {
            agent: Some(ConnectAgent::Claude)
        }
    );
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "grok".to_owned()].into_iter()).unwrap(),
        Command::Connect {
            agent: Some(ConnectAgent::Grok)
        }
    );
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "codex".to_owned()].into_iter()).unwrap(),
        Command::Connect {
            agent: Some(ConnectAgent::Codex)
        }
    );
}

#[test]
fn connect_agent_names_are_case_insensitive() {
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "CLAUDE".to_owned()].into_iter()).unwrap(),
        Command::Connect {
            agent: Some(ConnectAgent::Claude)
        }
    );
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "Grok".to_owned()].into_iter()).unwrap(),
        Command::Connect {
            agent: Some(ConnectAgent::Grok)
        }
    );
}

#[test]
fn connect_accepts_claude_code_alias_for_claude() {
    for alias in ["claude-code", "claude_code"] {
        assert_eq!(
            parse_args(vec!["connect".to_owned(), alias.to_owned()].into_iter()).unwrap(),
            Command::Connect {
                agent: Some(ConnectAgent::Claude)
            },
            "alias {alias:?} should map to Claude Code"
        );
    }
}

#[test]
fn connect_with_an_unknown_agent_returns_a_descriptive_error() {
    let err = parse_args(vec!["connect".to_owned(), "bogus".to_owned()].into_iter()).unwrap_err();
    assert!(
        err.contains("bogus"),
        "error should name the offending agent: {err:?}"
    );
    assert!(
        err.contains("claude") && err.contains("grok") && err.contains("codex"),
        "error should list the valid agents: {err:?}"
    );
}

#[test]
fn connect_agent_from_str_rejects_empty_and_garbage() {
    assert!(ConnectAgent::from_str("").is_err());
    assert!(ConnectAgent::from_str("not-an-agent").is_err());
    assert!(ConnectAgent::from_str("2").is_err());
}

#[test]
fn install_parses_as_its_own_command() {
    assert_eq!(
        parse_args(vec!["install".to_owned()].into_iter()).unwrap(),
        Command::Install
    );
}

#[test]
fn install_ignores_any_extra_arguments() {
    assert_eq!(
        parse_args(vec!["install".to_owned(), "--some-flag".to_owned()].into_iter()).unwrap(),
        Command::Install
    );
}
