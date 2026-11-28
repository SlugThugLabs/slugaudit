//! Tests for automatic grammar retry behavior and best-effort notices.

use crate::parse::fetch_gate::test_controls::{self, GateResetGuard};
use crate::sync::{ChangeStatus, FileChange, add_available_grammar_retries};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::collections::HashMap;

#[test]
fn uncached_untested_grammar_reports_best_effort_notice_and_retries_when_gate_open() -> TestResult {
    let _guard = GateResetGuard::new();
    let _failures_guard = crate::parse::local_load_failures::LocalLoadFailuresGuard::clear();
    test_controls::set_grammar_cached(false);

    let notices = crate::tools::untested_language_notices([("python", Some(2))]);
    ensure_eq(&notices.len(), &1usize, "one notice for python")?;
    let notice = notices.first().check("python notice")?;
    ensure_eq(
        notice,
        &"python: 2 files use an untested grammar. Only Rust is tested; treat python results as best-effort.",
        "notice states best-effort without mentioning downloads",
    )?;

    let unavailable = HashMap::from([("app.py".to_owned(), "python".to_owned())]);
    let current = vec![("app.py", "h1")];

    crate::parse::fetch_gate::record_fetch_failure().check("close gate")?;
    let mut during_cooldown = Vec::new();
    add_available_grammar_retries(&mut during_cooldown, &current, &unavailable)
        .check("retries during cooldown")?;
    ensure(
        during_cooldown.is_empty(),
        "uncached unavailable grammar waits while fetch gate is cooling down",
    )?;

    test_controls::elapse_cooldown();
    let mut after_cooldown = Vec::new();
    add_available_grammar_retries(&mut after_cooldown, &current, &unavailable)
        .check("retries after cooldown")?;
    ensure_eq(
        &after_cooldown,
        &vec![FileChange {
            relative_path: "app.py".to_owned(),
            status: ChangeStatus::Modified,
        }],
        "unavailable grammar is queued as Modified automatically once fetch gate is open",
    )?;
    Ok(())
}
