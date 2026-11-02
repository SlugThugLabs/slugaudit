use super::*;
use crate::connect_agents::find_agent;
use slugaudit_test_check::{Check, CheckErr, TestResult};

#[test]
fn an_atomic_write_for_a_path_without_a_parent_reports_the_home_unavailable() -> TestResult {
    let error = write_atomic(Path::new("/"), b"{}").check_err("root has no parent")?;

    assert!(
        matches!(error, ConnectError::HomeUnavailable),
        "expected HomeUnavailable, got {error:?}"
    );
    Ok(())
}

#[test]
fn disconnecting_without_a_config_home_reports_nothing_to_do() -> TestResult {
    let goose = find_agent("goose").check("goose is registered")?;

    let outcome = disconnect_agent(&goose, None).check("a missing home is not a failure")?;

    assert_eq!(outcome, DisconnectOutcome::NothingToDo);
    Ok(())
}

#[test]
fn disconnecting_an_entry_with_no_writable_section_or_cli_reports_nothing_to_do() -> TestResult {
    let temp = tempfile::tempdir().check("temp dir")?;

    for id in ["goose", "crush"] {
        let agent = find_agent(id).check(id)?;
        let outcome =
            disconnect_agent(&agent, Some(temp.path())).check("the no-op must not fail")?;

        assert_eq!(outcome, DisconnectOutcome::NothingToDo, "{id}");
    }
    Ok(())
}
