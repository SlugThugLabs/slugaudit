//! Tests for the plan checks (`crate::plan`): status-header freshness and
//! task descope.

use crate::plan::{check_plan_status_header, check_plan_task_descope, task_id_from_header};
use crate::test_support::{temp_root, write};

#[test]
fn status_header_accepts_an_audited_plan() {
    let root = temp_root();
    write(
        root.path(),
        ".planning/IMPLEMENTATION_PLAN.md",
        "# Plan\n\nStatus: implemented through Phase 12 (2026-08-10); see §22 for the 2026-08-11 audit corrections.\n",
    );
    let mut failures = Vec::new();
    check_plan_status_header(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn status_header_flags_a_pre_audit_plan() {
    let root = temp_root();
    write(
        root.path(),
        ".planning/IMPLEMENTATION_PLAN.md",
        "# Plan\n\nStatus: early-stage foundation only.\n",
    );
    let mut failures = Vec::new();
    check_plan_status_header(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("§22"), "{failures:?}");
}

#[test]
fn status_header_missing_line_is_flagged() {
    let root = temp_root();
    write(
        root.path(),
        ".planning/IMPLEMENTATION_PLAN.md",
        "# Plan\n\nNo status here.\n",
    );
    let mut failures = Vec::new();
    check_plan_status_header(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("no `Status:` line"), "{failures:?}");
}

#[test]
fn implemented_plan_task_needs_no_descope() {
    let root = temp_root();
    write(root.path(), "src/sync/reconcile.rs", "pub fn r() {}\n");
    write(
        root.path(),
        ".planning/IMPLEMENTATION_PLAN.md",
        "### Task 4.1 — Something\n\nFiles:\n\n- `src/sync/reconcile.rs`\n\nBehavior...\n",
    );
    write(
        root.path(),
        ".planning/DECISIONS.md",
        "# Decisions\n\n(no entries)\n",
    );
    let mut failures = Vec::new();
    check_plan_task_descope(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn unimplemented_plan_task_without_descope_is_flagged() {
    let root = temp_root();
    write(
        root.path(),
        ".planning/IMPLEMENTATION_PLAN.md",
        "### Task 5.2 — Constrained regex search\n\nFiles:\n\n- `src/search/regex.rs`\n\nBehavior...\n",
    );
    write(
        root.path(),
        ".planning/DECISIONS.md",
        "# Decisions\n\n(no descope for 5.2)\n",
    );
    let mut failures = Vec::new();
    check_plan_task_descope(root.path(), &mut failures);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("Task 5.2"), "{failures:?}");
}

#[test]
fn unimplemented_plan_task_with_descope_passes() {
    let root = temp_root();
    write(
        root.path(),
        ".planning/IMPLEMENTATION_PLAN.md",
        "### Task 5.2 — Constrained regex search\n\nFiles:\n\n- `src/search/regex.rs`\n\nBehavior...\n",
    );
    write(
        root.path(),
        ".planning/DECISIONS.md",
        "# Decisions\n\n## 2026-08-12 — descope\n\nTask 5.2 replaced by the query tool.\n",
    );
    let mut failures = Vec::new();
    check_plan_task_descope(root.path(), &mut failures);
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn task_id_from_header_parses_valid_and_rejects_invalid() {
    assert_eq!(
        task_id_from_header("### Task 5.1 — Implement bounded literal search").as_deref(),
        Some("Task 5.1")
    );
    assert_eq!(
        task_id_from_header("### Task 12.3 — Done thing").as_deref(),
        Some("Task 12.3")
    );
    assert!(task_id_from_header("### Task 5 — no sub number").is_none());
    assert!(task_id_from_header("## Phase 5 — not a task").is_none());
    assert!(task_id_from_header("### NotATask 5.1 — no").is_none());
}
