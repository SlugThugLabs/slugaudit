use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};

#[test]
fn no_arguments_means_serve() -> TestResult {
    ensure_eq(
        &parse_args(std::iter::empty()).check("valid arguments parse")?,
        &Command::Serve,
        "parse_args(std::iter::empty()).check(\"valid arguments parse\")? == Command::Serve",
    )?;
    Ok(())
}

#[test]
fn usage_with_a_plain_style_is_byte_identical_to_the_usage_text() -> TestResult {
    let rendered = render_usage(&Style::plain());

    ensure_eq(
        &rendered,
        &usage_text(),
        "plain help must match the usage text",
    )?;
    Ok(())
}

#[test]
fn usage_with_a_colored_style_keeps_every_heading_and_styles_it() -> TestResult {
    let rendered = render_usage(&Style::colored());

    for heading in ["USAGE:", "COMMANDS:", "OPTIONS:", "EXAMPLES:"] {
        ensure(
            rendered.contains(heading),
            format!("help must keep {heading:?}"),
        )?;
    }
    ensure(
        rendered.contains("\x1b[1m\x1b[36mCOMMANDS:\x1b[0m\x1b[0m"),
        format!("headings are bold cyan: {rendered:?}"),
    )?;
    Ok(())
}

#[test]
fn explicit_serve() -> TestResult {
    ensure_eq(
        &parse_args(vec!["serve".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Serve,
        "parse_args(vec![\"serve\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Serve",
    )?;
    Ok(())
}

#[test]
fn unrecognized_input_returns_a_descriptive_error_rather_than_silently_serving() -> TestResult {
    let err = parse_args(vec!["foobar".to_owned()].into_iter()).check_err("the call must fail")?;
    ensure(
        err.contains("unknown command"),
        "err.contains(\"unknown command\")",
    )?;
    ensure(err.contains("foobar"), "err.contains(\"foobar\")")?;
    ensure(
        err.contains("serve") && err.contains("connect") && err.contains("install"),
        "err.contains(\"serve\") && err.contains(\"connect\") && err.contains(\"install\")",
    )?;
    ensure(
        err.contains("menu") && err.contains("disconnect"),
        "err.contains(\"menu\") && err.contains(\"disconnect\")",
    )?;
    Ok(())
}

#[test]
fn explicit_help_command_parses_as_help() -> TestResult {
    ensure_eq(
        &parse_args(vec!["help".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Help,
        "parse_args(vec![\"help\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Help",
    )?;
    Ok(())
}

#[test]
fn case_mismatched_known_command_is_an_error() -> TestResult {
    let result = parse_args(vec!["SErve".to_owned()].into_iter());
    let err = result.check_err("case mismatched command should fail")?;
    ensure(
        err.contains("unknown command"),
        "err.contains(\"unknown command\")",
    )?;
    ensure(err.contains("SErve"), "err.contains(\"SErve\")")?;
    Ok(())
}

#[test]
fn connect_with_no_agent_picks_interactive() -> TestResult {
    ensure_eq(
        &parse_args(vec!["connect".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Connect { agent: None },
        "parse_args(vec![\"connect\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Connect { ag...",
    )?;
    Ok(())
}

#[test]
fn connect_accepts_agent_name() -> TestResult {
    ensure_eq(
        &parse_args(vec!["connect".to_owned(), "claude".to_owned()].into_iter())
            .check("valid arguments parse")?,
        &Command::Connect {
            agent: Some("claude".to_string()),
        },
        "parse_args(vec![\"connect\".to_owned(), \"claude\".to_owned()].into_iter()) .check(\"valid arguments parse\")? ==...",
    )?;
    Ok(())
}

#[test]
fn disconnect_with_no_agent_picks_interactive() -> TestResult {
    ensure_eq(
        &parse_args(vec!["disconnect".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Disconnect { agent: None },
        "parse_args(vec![\"disconnect\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Disconnec...",
    )?;
    Ok(())
}

#[test]
fn disconnect_accepts_agent_name() -> TestResult {
    ensure_eq(
        &parse_args(vec!["disconnect".to_owned(), "agy".to_owned()].into_iter())
            .check("valid arguments parse")?,
        &Command::Disconnect {
            agent: Some("agy".to_string()),
        },
        "parse_args(vec![\"disconnect\".to_owned(), \"agy\".to_owned()].into_iter()) .check(\"valid arguments parse\")? ==...",
    )?;
    Ok(())
}

#[test]
fn remove_alias_parses_as_disconnect() -> TestResult {
    ensure_eq(
        &parse_args(vec!["remove".to_owned(), "agy".to_owned()].into_iter())
            .check("valid arguments parse")?,
        &Command::Disconnect {
            agent: Some("agy".to_string()),
        },
        "parse_args(vec![\"remove\".to_owned(), \"agy\".to_owned()].into_iter()) .check(\"valid arguments parse\")? == Com...",
    )?;
    Ok(())
}

#[test]
fn install_parses_as_its_own_command() -> TestResult {
    ensure_eq(
        &parse_args(vec!["install".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Install,
        "parse_args(vec![\"install\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Install",
    )?;
    Ok(())
}

#[test]
fn parse_install_without_args_ok() -> TestResult {
    ensure_eq(
        &parse_args(vec!["install".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Install,
        "parse_args(vec![\"install\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Install",
    )?;
    Ok(())
}

#[test]
fn parse_install_rejects_extra_args() -> TestResult {
    let err = parse_args(vec!["install".to_owned(), "--something".to_owned()].into_iter())
        .check_err("the call must fail")?;
    ensure_eq(
        &err,
        &"`install` takes no arguments; unexpected argument \"--something\"",
        "err == \"`install` takes no arguments; unexpected argument \\\"--something\\\"\"",
    )?;
    Ok(())
}

#[test]
fn menu_parses_as_its_own_command() -> TestResult {
    ensure_eq(
        &parse_args(vec!["menu".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Menu,
        "parse_args(vec![\"menu\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Menu",
    )?;
    Ok(())
}

#[test]
fn parse_menu_without_args_ok() -> TestResult {
    ensure_eq(
        &parse_args(vec!["menu".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Menu,
        "parse_args(vec![\"menu\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Menu",
    )?;
    Ok(())
}

#[test]
fn parse_menu_rejects_extra_args() -> TestResult {
    let err = parse_args(vec!["menu".to_owned(), "--wat".to_owned()].into_iter())
        .check_err("the call must fail")?;
    ensure_eq(
        &err,
        &"`menu` takes no arguments; unexpected argument \"--wat\"",
        "err == \"`menu` takes no arguments; unexpected argument \\\"--wat\\\"\"",
    )?;
    Ok(())
}

#[test]
fn unrecognized_input_lists_menu_among_the_known_commands() -> TestResult {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).check_err("the call must fail")?;
    ensure(
        err.contains("menu"),
        format!("the error should name menu: {err}"),
    )?;
    Ok(())
}

#[test]
fn update_parses_as_its_own_command() -> TestResult {
    ensure_eq(
        &parse_args(vec!["update".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Update,
        "parse_args(vec![\"update\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Update",
    )?;
    Ok(())
}

#[test]
fn unrecognized_input_lists_update_among_the_known_commands() -> TestResult {
    let err = parse_args(vec!["bogus".to_owned()].into_iter()).check_err("the call must fail")?;
    ensure(
        err.contains("update"),
        format!("the error should name update: {err}"),
    )?;
    Ok(())
}

#[test]
fn version_parses_in_all_three_spellings() -> TestResult {
    for arg in ["version", "--version", "-V"] {
        ensure_eq(
            &parse_args(vec![arg.to_owned()].into_iter()).check("valid arguments parse")?,
            &Command::Version,
            format!("{arg} should parse as Version"),
        )?;
    }
    Ok(())
}

#[test]
fn help_names_the_installed_binary_connect_registers() -> TestResult {
    ensure(
        usage_text().contains("Register the installed SlugAudit binary"),
        "connect prefers the installed binary, so the help must say so",
    )?;
    ensure(
        !usage_text().contains("Register this binary"),
        "the running binary is only used when no installed copy exists",
    )?;
    Ok(())
}

#[test]
fn cli_exit_lowercase_v_parses_as_version() -> TestResult {
    ensure_eq(
        &parse_args(vec!["-v".to_owned()].into_iter()).check("valid arguments parse")?,
        &Command::Version,
        "parse_args(vec![\"-v\".to_owned()].into_iter()).check(\"valid arguments parse\")? == Command::Version",
    )?;
    Ok(())
}

#[test]
fn cli_exit_connect_with_an_extra_argument_is_a_usage_error() -> TestResult {
    let err = parse_args(vec!["connect".to_owned(), "a".to_owned(), "b".to_owned()].into_iter())
        .check_err("the call must fail")?;
    ensure_eq(
        &err,
        &"`connect` takes at most one agent name; unexpected argument \"b\"",
        "err == \"`connect` takes at most one agent name; unexpected argument \\\"b\\\"\"",
    )?;
    Ok(())
}

#[test]
fn cli_exit_disconnect_and_remove_with_an_extra_argument_are_usage_errors() -> TestResult {
    for word in ["disconnect", "remove"] {
        let err = parse_args(vec![word.to_owned(), "a".to_owned(), "b".to_owned()].into_iter())
            .check_err("the call must fail")?;
        ensure(
            err.starts_with(&format!("`{word}` takes at most one agent name")),
            &err,
        )?;
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
                ensure_eq(
                    &parse(&args).check("help parses")?,
                    &Command::CommandHelp(topic),
                    format!("{args:?}"),
                )?;
            }
        }
    }
    Ok(())
}

#[test]
fn update_help_never_parses_to_the_update_command() -> TestResult {
    for args in [["update", "--help"], ["update", "-h"]] {
        ensure_ne(
            &parse(&args).check("help parses")?,
            &Command::Update,
            "parse(&args).check(\"help parses\")? != Command::Update",
        )?;
    }
    Ok(())
}

#[test]
fn every_subcommand_has_its_own_usage_naming_that_subcommand() -> TestResult {
    for (topic, needle) in [
        (Topic::Serve, "slugaudit serve"),
        (Topic::Connect, "slugaudit connect [AGENT]"),
        (Topic::Disconnect, "slugaudit disconnect [AGENT]"),
        (Topic::Install, "slugaudit install"),
        (Topic::Menu, "slugaudit menu"),
        (Topic::Update, "slugaudit update"),
    ] {
        let text = topic_usage(topic);
        ensure(text.starts_with("USAGE:\n"), &text)?;
        ensure(text.contains(needle), format!("{needle}: {text}"))?;
        ensure(text.contains("-h, --help"), &text)?;
    }
    Ok(())
}

#[test]
fn general_help_lists_serve_and_shares_every_description_with_the_subcommand_help() -> TestResult {
    let general = usage_text();
    ensure(
        general.contains(
            "    slugaudit serve                  Run the MCP server (stdio transport)\n",
        ),
        &general,
    )?;
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
            ensure(
                general.contains(line),
                format!("general help lacks {line:?}"),
            )?;
        }
    }
    Ok(())
}

#[path = "cli_error_tests.rs"]
mod cli_error_tests;
