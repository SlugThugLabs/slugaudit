use super::*;
use slugaudit_test_check::{Check, TestResult, ensure};

#[test]
fn claim_report_note_is_scoped_per_project_and_resets_on_clear() -> TestResult {
    let session = SessionMechanics::default();
    let shared = session.clone();
    let root_a = Path::new("/session-mechanics-test/a");
    let root_b = Path::new("/session-mechanics-test/b");

    ensure(
        session.claim_report_note(root_a).check("first claim a")?,
        "first claim on project A delivers the note",
    )?;
    ensure(
        !shared.claim_report_note(root_a).check("second claim a")?,
        "second claim on project A does not repeat the note",
    )?;
    ensure(
        shared.claim_report_note(root_b).check("first claim b")?,
        "project B has its own independent report note claim",
    )?;
    session.clear_project(root_a).check("clear project a")?;
    ensure(
        session
            .claim_report_note(root_a)
            .check("claim after clear")?,
        "clear_project resets the claim for project A",
    )
}
