//! Tests for `Check` / `CheckErr`, written as an external consumer so the
//! recorded location is proven across the crate boundary.

use slugaudit_test_check::{Check, CheckErr, CheckSources, TestResult};

#[test]
fn check_returns_the_value_of_an_ok_result() -> TestResult {
    let result: Result<u8, String> = Ok(7);
    let value = result.check("read the value")?;
    assert_eq!(value, 7);
    Ok(())
}

#[test]
fn check_turns_an_err_into_a_test_error_with_the_display_cause() -> TestResult {
    let failing: Result<u8, String> = Err("disk on fire".to_owned());
    let line = line!() + 1;
    let outcome = failing.check("open the disk");
    let error = outcome.err().check("an Err must become a TestError")?;
    assert_eq!(
        format!("{error:?}"),
        format!("{}:{line}: open the disk: disk on fire", file!())
    );
    Ok(())
}

#[test]
fn check_returns_the_value_of_some() -> TestResult {
    let value = Some(3_u8).check("first item")?;
    assert_eq!(value, 3);
    Ok(())
}

#[test]
fn check_turns_none_into_a_test_error() -> TestResult {
    let absent: Option<u8> = None;
    let line = line!() + 1;
    let outcome = absent.check("first item");
    let error = outcome.err().check("None must become a TestError")?;
    assert_eq!(
        format!("{error:?}"),
        format!("{}:{line}: first item: value was absent", file!())
    );
    Ok(())
}

#[test]
fn check_err_returns_the_error_value() -> TestResult {
    let result: Result<u8, String> = Err("boom".to_owned());
    let error = result.check_err("expect a failure")?;
    assert_eq!(error, "boom");
    Ok(())
}

#[test]
fn check_err_on_ok_reports_the_value() -> TestResult {
    let result: Result<u8, String> = Ok(9);
    let line = line!() + 1;
    let outcome = result.check_err("expect a failure");
    let error = outcome.err().check("Ok must become a TestError")?;
    assert_eq!(
        format!("{error:?}"),
        format!(
            "{}:{line}: expect a failure: expected an error, got Ok(9)",
            file!()
        )
    );
    Ok(())
}

#[test]
fn display_matches_debug() -> TestResult {
    let result: Result<u8, String> = Err("x".to_owned());
    let error = result.check("step").err().check("must fail")?;
    assert_eq!(format!("{error}"), format!("{error:?}"));
    Ok(())
}

#[test]
fn the_recorded_line_is_the_callers_line_through_the_trait() -> TestResult {
    let result: Result<u8, String> = Err("e".to_owned());
    let (outcome, line) = (result.check("same-line probe"), line!());
    let error = outcome.err().check("must fail")?;
    assert_eq!(
        format!("{error:?}"),
        format!("{}:{line}: same-line probe: e", file!())
    );
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
    assert_eq!(
        format!("{error:?}"),
        format!(
            "{}:{line}: run the step: outer failed: caused by: inner failed: caused by: root cause",
            file!()
        )
    );
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
    assert_eq!(
        format!("{error:?}"),
        format!("{}:{line}: run the step: wrapper: root cause", file!())
    );
    Ok(())
}

#[test]
fn check_sources_returns_the_value_of_an_ok_result() -> TestResult {
    let result: Result<u8, Root> = Ok(5);
    assert_eq!(result.check_sources("read the value")?, 5);
    Ok(())
}
