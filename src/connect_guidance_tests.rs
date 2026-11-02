use super::*;
use crate::connect_agents::find_agent;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn a_rejected_add_points_at_the_client_output_instead_of_an_install() -> TestResult {
    let pi = find_agent("pi").check("pi is registered")?;
    let attempted = "pi mcp add slugaudit -- /bin/slugaudit";

    let lines = failure_lines(
        &pi,
        Path::new("/bin/slugaudit"),
        attempted,
        Some(attempted),
        "exit status: 1",
    );
    let text = lines.join("\n");

    assert!(text.contains("rejected the registration"), "{text}");
    assert!(text.contains("read the failure above"), "{text}");
    assert!(
        !text.contains("install `pi`"),
        "an installed CLI must not be described as missing: {text}"
    );
    Ok(())
}

#[test]
fn only_an_absent_cli_is_advised_to_install() -> TestResult {
    let pi = find_agent("pi").check("pi is registered")?;

    let error = cli_absent(
        &pi,
        Path::new("/bin/slugaudit"),
        "pi",
        "pi mcp add slugaudit -- /bin/slugaudit",
    );
    let text = error.to_string();

    assert!(text.contains("`pi` was not found on PATH"), "{text}");
    assert!(text.contains("install `pi`"), "{text}");
    assert!(!text.contains("rejected the registration"), "{text}");
    Ok(())
}

#[test]
fn the_fallback_note_reads_as_one_sentence_for_each_reason() -> TestResult {
    let pi = find_agent("pi").check("pi is registered")?;
    let home = Path::new("/home/u");
    let written = target(&pi, Some(home));

    let absent = fallback_note(&pi, Some(home), "pi", "was not found on PATH");
    let rejected = fallback_note(
        &pi,
        Some(home),
        "pi",
        "rejected `mcp add` (`pi mcp add` exited with exit status: 1)",
    );

    assert_eq!(
        absent,
        format!("`pi` was not found on PATH, so slugaudit wrote {written} instead.")
    );
    assert_eq!(
        rejected,
        format!(
            "`pi` rejected `mcp add` (`pi mcp add` exited with exit status: 1), \
             so slugaudit wrote {written} instead."
        )
    );
    Ok(())
}
