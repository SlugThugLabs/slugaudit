//! Tests for `Check` / `CheckErr`, written as an external consumer so the
//! recorded location is proven across the crate boundary.

use slugaudit_test_check::{Check, CheckErr, TestResult};

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
