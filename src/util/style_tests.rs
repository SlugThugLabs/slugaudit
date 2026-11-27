use super::*;
use slugaudit_test_check::{TestResult, ensure};
use std::ffi::OsString;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[test]
fn style_colours_a_terminal_with_a_clean_environment() -> TestResult {
    ensure(
        Style::detect(true, env(&[("TERM", "xterm-256color")])).enabled(),
        "terminal with clean environment enables color",
    )
}

#[test]
fn style_is_disabled_off_a_terminal() -> TestResult {
    ensure(
        !Style::detect(false, env(&[])).enabled(),
        "non-terminal disables color",
    )
}

#[test]
fn style_is_disabled_by_a_non_empty_no_color() -> TestResult {
    ensure(
        !Style::detect(true, env(&[("NO_COLOR", "1")])).enabled(),
        "non-empty NO_COLOR disables color",
    )
}

#[test]
fn style_ignores_an_empty_no_color() -> TestResult {
    ensure(
        Style::detect(true, env(&[("NO_COLOR", "")])).enabled(),
        "empty NO_COLOR leaves terminal color enabled",
    )
}

#[test]
fn style_is_disabled_by_term_dumb() -> TestResult {
    ensure(
        !Style::detect(true, env(&[("TERM", "dumb")])).enabled(),
        "TERM=dumb disables color",
    )
}
