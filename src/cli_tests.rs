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
    let styled = usage();
    if styled.contains('\x1b') {
        for heading in ["USAGE:", "COMMANDS:", "OPTIONS:", "EXAMPLES:"] {
            assert!(styled.contains(heading), "help must keep {heading:?}");
        }
    } else {
        assert_eq!(styled, USAGE, "non-terminal help must match USAGE exactly");
    }
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
    assert!(result.is_err());
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
