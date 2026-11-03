//! Tests for `Check` / `CheckErr`, written as an external consumer so the
//! recorded location is proven across the crate boundary.

use slugaudit_test_check::{
    Check, CheckErr, CheckSources, TestResult, ensure, ensure_eq, ensure_ne,
};

#[test]
fn check_returns_the_value_of_an_ok_result() -> TestResult {
    let result: Result<u8, String> = Ok(7);
    let value = result.check("read the value")?;
    ensure_eq(value, 7, "the values match")?;
    Ok(())
}

#[test]
fn check_turns_an_err_into_a_test_error_with_the_display_cause() -> TestResult {
    let failing: Result<u8, String> = Err("disk on fire".to_owned());
    let line = line!() + 1;
    let outcome = failing.check("open the disk");
    let error = outcome.err().check("an Err must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!("{}:{line}: open the disk: disk on fire", file!()),
        "the values match",
    )?;
    Ok(())
}

#[test]
fn check_returns_the_value_of_some() -> TestResult {
    let value = Some(3_u8).check("first item")?;
    ensure_eq(value, 3, "the values match")?;
    Ok(())
}

#[test]
fn check_turns_none_into_a_test_error() -> TestResult {
    let absent: Option<u8> = None;
    let line = line!() + 1;
    let outcome = absent.check("first item");
    let error = outcome.err().check("None must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!("{}:{line}: first item: value was absent", file!()),
        "the values match",
    )?;
    Ok(())
}

#[test]
fn check_err_returns_the_error_value() -> TestResult {
    let result: Result<u8, String> = Err("boom".to_owned());
    let error = result.check_err("expect a failure")?;
    ensure_eq(error, "boom", "the values match")?;
    Ok(())
}

#[test]
fn check_err_on_ok_reports_the_value() -> TestResult {
    let result: Result<u8, String> = Ok(9);
    let line = line!() + 1;
    let outcome = result.check_err("expect a failure");
    let error = outcome.err().check("Ok must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!(
            "{}:{line}: expect a failure: expected an error, got Ok(9)",
            file!()
        ),
        "the values match",
    )?;
    Ok(())
}

#[test]
fn display_matches_debug() -> TestResult {
    let result: Result<u8, String> = Err("x".to_owned());
    let error = result.check("step").err().check("must fail")?;
    ensure_eq(format!("{error}"), format!("{error:?}"), "the values match")?;
    Ok(())
}

#[test]
fn the_recorded_line_is_the_callers_line_through_the_trait() -> TestResult {
    let result: Result<u8, String> = Err("e".to_owned());
    let (outcome, line) = (result.check("same-line probe"), line!());
    let error = outcome.err().check("must fail")?;
    ensure_eq(
        format!("{error:?}"),
        format!("{}:{line}: same-line probe: e", file!()),
        "the values match",
    )?;
    Ok(())
}

/// A three-level error whose `Display` hides its cause, as `#[error("...")]`
/// with a `#[source]` field does.
#[derive(Debug)]
struct Outer(Inner);

#[derive(Debug)]
struct Inner(Root);

#[derive(Debug)]
struct Root;

impl std::fmt::Display for Outer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "outer failed")
    }
}

impl std::fmt::Display for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inner failed")
    }
}

impl std::fmt::Display for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "root cause")
    }
}

impl std::error::Error for Root {}

impl std::error::Error for Inner {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl std::error::Error for Outer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn check_sources_walks_the_whole_source_chain_into_the_cause() -> TestResult {
    let failing: Result<u8, Outer> = Err(Outer(Inner(Root)));
    let line = line!() + 1;
    let outcome = failing.check_sources("run the step");
    let error = outcome.err().check("an Err must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!(
            "{}:{line}: run the step: outer failed: caused by: inner failed: caused by: root cause",
            file!()
        ),
        "the values match",
    )?;
    Ok(())
}

/// An error whose `Display` already embeds its source.
#[derive(Debug)]
struct Embedding(Root);

impl std::fmt::Display for Embedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wrapper: {}", self.0)
    }
}

impl std::error::Error for Embedding {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn check_sources_does_not_repeat_a_source_the_display_already_names() -> TestResult {
    let failing: Result<u8, Embedding> = Err(Embedding(Root));
    let line = line!() + 1;
    let outcome = failing.check_sources("run the step");
    let error = outcome.err().check("an Err must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!("{}:{line}: run the step: wrapper: root cause", file!()),
        "the values match",
    )?;
    Ok(())
}

#[test]
fn check_sources_returns_the_value_of_an_ok_result() -> TestResult {
    let result: Result<u8, Root> = Ok(5);
    ensure_eq(
        result.check_sources("read the value")?,
        5,
        "the values match",
    )?;
    Ok(())
}

#[test]
fn ensure_passes_on_true() -> TestResult {
    ensure(1 + 1 == 2, "arithmetic holds")
}

#[test]
fn ensure_fails_with_the_callers_file_and_line() -> TestResult {
    let line = line!() + 1;
    let outcome = ensure(false, format!("flag {} set", "was not"));
    let error = outcome.err().check("false must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!("{}:{line}: flag was not set: condition was false", file!()),
        "ensure failure text",
    )
}

#[test]
fn ensure_eq_passes_on_equal_values_and_mixed_references() -> TestResult {
    let value = 4_u8;
    ensure_eq(value, 4_u8, "owned values")?;
    ensure_eq(&value, &4_u8, "references")?;
    ensure_eq(String::from("a"), "a", "String against str")
}

#[test]
fn ensure_eq_fails_with_location_and_both_values() -> TestResult {
    let line = line!() + 1;
    let outcome = ensure_eq(1_u8, 2_u8, "the counts");
    let error = outcome.err().check("unequal must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!(
            "{}:{line}: the counts: expected left == right\nleft: 1\nright: 2",
            file!()
        ),
        "ensure_eq failure text",
    )
}

#[test]
fn ensure_ne_passes_on_different_values() -> TestResult {
    ensure_ne(1_u8, 2_u8, "different values")?;
    ensure_ne("a", "b", "different strs")
}

#[test]
fn ensure_ne_fails_with_location_and_both_values() -> TestResult {
    let line = line!() + 1;
    let outcome = ensure_ne("x", "x", "the names");
    let error = outcome.err().check("equal must become a TestError")?;
    ensure_eq(
        format!("{error:?}"),
        format!(
            "{}:{line}: the names: expected left != right\nleft: \"x\"\nright: \"x\"",
            file!()
        ),
        "ensure_ne failure text",
    )
}
