use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn a_zero_budget_is_exceeded_immediately() -> TestResult {
    let deadline = Deadline::after(Duration::ZERO);
    deadline
        .exceeded()
        .check("zero deadline is immediately exceeded")?;
    Ok(())
}

#[test]
fn a_generous_budget_is_not_exceeded_immediately() -> TestResult {
    let deadline = Deadline::after(Duration::from_mins(1));
    ensure(
        deadline.exceeded().is_none(),
        "1-minute deadline is not exceeded immediately",
    )
}

#[test]
fn a_spent_budget_has_no_milliseconds_remaining() -> TestResult {
    let deadline = Deadline::after(Duration::ZERO);
    ensure_eq(
        &deadline.remaining_millis(),
        &None,
        "spent deadline has None remaining milliseconds",
    )
}

#[test]
fn a_sub_millisecond_remainder_counts_as_spent() -> TestResult {
    let deadline = Deadline::after(Duration::from_micros(999));
    ensure_eq(
        &deadline.remaining_millis(),
        &None,
        "sub-millisecond remainder counts as spent",
    )
}

#[test]
fn a_live_budget_reports_its_whole_remaining_milliseconds() -> TestResult {
    let deadline = Deadline::after(Duration::from_mins(1));
    let remaining = deadline.remaining_millis().check("time is left")?;
    ensure(
        remaining.get() > 59_000 && remaining.get() <= 60_000,
        format!("{remaining}"),
    )
}

#[test]
fn a_cap_below_the_remaining_budget_is_a_unit_bound() -> TestResult {
    let operation = Deadline::after(Duration::from_mins(1));
    let (unit, bound) = operation.capped(Duration::from_secs(5));
    ensure_eq(&bound, &BudgetBound::Unit, "unit cap wins when smaller")?;
    let remaining = unit.remaining_millis().check("time is left")?.get();
    ensure((4_000..=5_000).contains(&remaining), format!("{remaining}"))
}

#[test]
fn a_cap_above_the_remaining_budget_is_an_operation_bound() -> TestResult {
    let operation = Deadline::after(Duration::from_secs(5));
    let (unit, bound) = operation.capped(Duration::from_mins(1));
    ensure_eq(
        &bound,
        &BudgetBound::Operation,
        "operation cap wins when smaller",
    )?;
    let remaining = unit.remaining_millis().check("time is left")?.get();
    ensure(remaining <= 5_000, format!("{remaining}"))
}

#[test]
fn a_spent_operation_is_an_operation_bound_whatever_the_cap() -> TestResult {
    let operation = Deadline::after(Duration::ZERO);
    let (unit, bound) = operation.capped(Duration::from_secs(5));
    ensure_eq(
        &bound,
        &BudgetBound::Operation,
        "spent operation stays Operation bound",
    )?;
    unit.exceeded().check("unit is exceeded")?;
    Ok(())
}

#[test]
fn a_zero_cap_is_a_spent_unit_bound() -> TestResult {
    let operation = Deadline::after(Duration::from_mins(1));
    let (unit, bound) = operation.capped(Duration::ZERO);
    ensure_eq(&bound, &BudgetBound::Unit, "zero unit cap is Unit bound")?;
    unit.exceeded().check("unit is exceeded")?;
    Ok(())
}
