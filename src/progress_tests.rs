use super::exact_fraction as sampling_fraction;

#[test]
fn an_unknown_total_reports_zero() {
    assert!(sampling_fraction(5, 0).expect("zero total").abs() < f64::EPSILON);
}

#[test]
fn a_sample_reports_its_share_of_the_total() {
    assert!((sampling_fraction(1, 4).expect("in range") - 0.25).abs() < f64::EPSILON);
    assert!((sampling_fraction(4, 4).expect("in range") - 1.0).abs() < f64::EPSILON);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn a_count_beyond_u32_is_an_error_not_a_rounded_fraction() {
    let too_big = usize::try_from(u64::from(u32::MAX) + 1).expect("64-bit usize");

    assert!(sampling_fraction(too_big, 10).is_err());
    assert!(sampling_fraction(1, too_big).is_err());
}
