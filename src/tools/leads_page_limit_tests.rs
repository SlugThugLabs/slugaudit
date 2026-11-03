use super::*;
use slugaudit_test_check::{TestResult, ensure_eq};

#[test]
fn limited_by_names_the_cause_for_every_combination() -> TestResult {
    let size = |at_ceiling| PageSize {
        bytes: 10,
        at_ceiling,
    };
    for stopped_by in [None, Some(PageLimit::Limit), Some(PageLimit::PageBytes)] {
        for at_ceiling in [false, true] {
            ensure_eq(
                &limited_by(true, stopped_by, size(at_ceiling)),
                &PageLimit::None,
                "limited_by(true, stopped_by, size(at_ceiling)) == PageLimit::None",
            )?;
        }
    }
    for at_ceiling in [false, true] {
        ensure_eq(
            &limited_by(false, Some(PageLimit::Limit), size(at_ceiling)),
            &PageLimit::Limit,
            "limited_by(false, Some(PageLimit::Limit), size(at_ceiling)) == PageLimit::Limit",
        )?;
    }
    for stopped_by in [None, Some(PageLimit::PageBytes)] {
        ensure_eq(
            &limited_by(false, stopped_by, size(false)),
            &PageLimit::PageBytes,
            "limited_by(false, stopped_by, size(false)) == PageLimit::PageBytes",
        )?;
        ensure_eq(
            &limited_by(false, stopped_by, size(true)),
            &PageLimit::Ceiling,
            "limited_by(false, stopped_by, size(true)) == PageLimit::Ceiling",
        )?;
    }
    Ok(())
}
