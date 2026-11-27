use super::*;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn a_session_profile_is_per_project_and_the_last_choice_wins() -> TestResult {
    let first = Path::new("/session-profile-test/first");
    let second = Path::new("/session-profile-test/second");
    ensure_eq(
        &session_profile(first).check("read first")?,
        &None,
        "unset project starts with None session profile",
    )?;

    set_session_profile(first, AuditProfile::Lean).check("choose lean")?;
    set_session_profile(first, AuditProfile::Audit).check("choose audit")?;

    ensure_eq(
        &session_profile(first).check("read first")?,
        &Some(AuditProfile::Audit),
        "latest session profile wins for project",
    )?;
    ensure_eq(
        &session_profile(second).check("read second")?,
        &None,
        "other project stays unaffected",
    )
}
