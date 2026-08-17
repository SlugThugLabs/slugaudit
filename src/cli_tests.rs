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
fn unrecognized_input_returns_a_descriptive_error_rather_than_silently_serving() {
    let err = parse_args(vec!["foobar".to_owned()].into_iter()).unwrap_err();
    assert!(err.contains("unknown command"));
    assert!(err.contains("foobar"));
    assert!(err.contains("serve") && err.contains("connect") && err.contains("install"));
}

#[test]
fn explicit_help_command_parses_as_help() {
    assert_eq!(
        parse_args(vec!["help".to_owned()].into_iter()).unwrap(),
        Command::Help
    );
}

#[test]
fn case_mismatched_known_command_is_an_error() {
    // Edge case: a user runs `slugaudit SErve` (mixed case). Currently
    // the parser is case-sensitive — we don't normalize to lowercase.
    // Record the behavior so future case-insensitive work is a
    // deliberate change, not a quiet backward-incompat.
    let result = parse_args(vec!["SErve".to_owned()].into_iter());
    assert!(
        result.is_err(),
        "case-sensitive commands are the documented behavior; the test name and assertion must agree"
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
    for (name, expected) in [
        ("claude", ConnectAgent::Claude),
        ("grok", ConnectAgent::Grok),
        ("codex", ConnectAgent::Codex),
    ] {
        assert_eq!(
            parse_args(vec!["connect".to_owned(), name.to_owned()].into_iter()).unwrap(),
            Command::Connect {
                agent: Some(expected)
            }
        );
    }
}

#[test]
fn connect_agent_names_are_case_insensitive() {
    assert_eq!(
        parse_args(vec!["connect".to_owned(), "CLAUDE".to_owned()].into_iter()).unwrap(),
        Command::Connect {
            agent: Some(ConnectAgent::Claude)
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
            }
        );
    }
}

#[test]
fn connect_with_an_unknown_agent_returns_a_descriptive_error() {
    let err = parse_args(vec!["connect".to_owned(), "unknown".to_owned()].into_iter()).unwrap_err();
    assert!(err.contains("unknown"));
    assert!(err.contains("claude") && err.contains("grok") && err.contains("codex"));
}

#[test]
fn connect_agent_from_str_rejects_empty_and_garbage() {
    assert!(ConnectAgent::from_str("").is_err());
    assert!(ConnectAgent::from_str("foobar").is_err());
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
        parse_args(vec!["install".to_owned(), "--something".to_owned()].into_iter()).unwrap(),
        Command::Install
    );
}

#[test]
fn menu_parses_as_its_own_command() {
    assert_eq!(
        parse_args(vec!["menu".to_owned()].into_iter()).unwrap(),
        Command::Menu
    );
}

#[test]
fn menu_ignores_any_extra_arguments() {
    assert_eq!(
        parse_args(vec!["menu".to_owned(), "--wat".to_owned()].into_iter()).unwrap(),
        Command::Menu
    );
}

#[test]
fn unrecognized_input_lists_menu_among_the_known_commands() {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).unwrap_err();
    assert!(err.contains("menu"), "the error should name menu: {err}");
}

#[test]
fn version_parses_in_all_three_spellings() {
    for arg in ["version", "--version", "-V"] {
        assert_eq!(
            parse_args(vec![arg.to_owned()].into_iter()).unwrap(),
            Command::Version,
            "{arg} should parse as Version"
        );
    }
}
