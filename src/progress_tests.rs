use super::exact_fraction as sampling_fraction;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn an_unknown_total_reports_zero() -> TestResult {
    assert!(sampling_fraction(5, 0).check("zero total")?.abs() < f64::EPSILON);
    Ok(())
}

#[test]
fn a_sample_reports_its_share_of_the_total() -> TestResult {
    assert!((sampling_fraction(1, 4).check("in range")? - 0.25).abs() < f64::EPSILON);
    assert!((sampling_fraction(4, 4).check("in range")? - 1.0).abs() < f64::EPSILON);
    Ok(())
}

#[test]
#[cfg(target_pointer_width = "64")]
fn a_count_beyond_u32_is_an_error_not_a_rounded_fraction() -> TestResult {
    let too_big = usize::try_from(u64::from(u32::MAX) + 1).check("64-bit usize")?;

    assert!(sampling_fraction(too_big, 10).is_err());
    assert!(sampling_fraction(1, too_big).is_err());
    Ok(())
}
