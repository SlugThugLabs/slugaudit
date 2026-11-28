use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn grammar_cli_validates_aliases_prose_and_unknown_languages() -> TestResult {
    let alias_err = validate_install_target("shell").check_err("alias rejected")?;
    ensure(
        alias_err.to_string().contains("bash"),
        "alias error names canonical language",
    )?;

    let prose_err = validate_install_target("markdown").check_err("prose rejected")?;
    ensure(
        prose_err.to_string().contains("prose"),
        "prose error explains refusal",
    )?;

    let unknown_err =
        validate_install_target("no_such_lang_xyz").check_err("unknown language rejected")?;
    ensure(
        unknown_err.to_string().contains("unknown language"),
        "unknown language error",
    )?;

    let ok = validate_install_target("  rust  ").check("rust valid")?;
    ensure_eq(&ok, &"rust", "trims surrounding whitespace")?;
    Ok(())
}

#[test]
fn grammar_cli_install_rust_is_already_installed_without_prompt() -> TestResult {
    run_grammar(GrammarAction::Install {
        language: "rust".to_owned(),
        yes: false,
    })
    .check("bundled rust succeeds without --yes")?;
    Ok(())
}

#[test]
fn grammar_cli_list_succeeds() -> TestResult {
    run_grammar(GrammarAction::List).check("grammar list succeeds")?;
    Ok(())
}

#[test]
fn grammar_cli_non_tty_install_without_yes_declines() -> TestResult {
    use std::io::IsTerminal as _;
    if std::io::stdin().is_terminal() {
        return Ok(());
    }
    if tree_sitter_language_pack::has_parser("zig") {
        return Ok(());
    }
    let err = run_grammar(GrammarAction::Install {
        language: "zig".to_owned(),
        yes: false,
    })
    .check_err("non-tty without --yes must decline")?;
    ensure(
        matches!(err, GrammarCliError::Declined(_)),
        "returns Declined on non-tty without --yes",
    )?;
    Ok(())
}
