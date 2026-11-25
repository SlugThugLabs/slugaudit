use super::{StoredRow, add_classification_drift_changes, filter_skipped_records, load_failed};
use crate::model::SourceIdentity;
use crate::sync::manifest::{ChangeStatus, FileChange};
use crate::sync::revision_skipped::{SKIP_REASON_OVERSIZE, SKIP_REASON_UNREADABLE, SkippedRecord};
use slugaudit_test_check::{Check, TestResult, ensure_eq};
use std::collections::HashMap;

fn skipped_record(path: &str, reason: &str) -> SkippedRecord {
    SkippedRecord {
        relative_path: path.to_owned(),
        reason: reason.to_owned(),
        identity: SourceIdentity::new(path.to_owned(), "stat-h".to_owned()),
        byte_len: Some(128),
        modified_unix_seconds: Some(10),
    }
}

#[test]
fn add_load_failed_retries_skips_locally_failed_languages() -> TestResult {
    crate::parse::fetch_gate::record_local_load_failure("rust")
        .check("record local failure for rust")?;
    let load_failed_map = HashMap::from([
        ("a.rs".to_owned(), Some("rust".to_owned())),
        ("b.py".to_owned(), Some("python".to_owned())),
    ]);
    let current = vec![("a.rs", "h1"), ("b.py", "h2")];
    let mut changes = Vec::new();

    load_failed::add_load_failed_retries(&mut changes, &current, &load_failed_map)
        .check("add load failed retries")?;

    ensure_eq(
        &changes.len(),
        &1usize,
        "only non-locally-failed language retried",
    )?;
    ensure_eq(
        &changes.first().map(|c| c.relative_path.as_str()),
        &Some("b.py"),
        "python file is retried while rust file is skipped",
    )?;
    Ok(())
}

#[test]
fn classification_drift_detects_skip_reason_change_with_unchanged_hash() -> TestResult {
    let stored = vec![
        StoredRow {
            path: "a.bin".to_owned(),
            content_hash: "stat-h".to_owned(),
            file_kind: "skipped".to_owned(),
            skip_reason: Some(SKIP_REASON_OVERSIZE.to_owned()),
            content_is_null: true,
        },
        StoredRow {
            path: "b.bin".to_owned(),
            content_hash: "stat-h".to_owned(),
            file_kind: "skipped".to_owned(),
            skip_reason: Some(SKIP_REASON_OVERSIZE.to_owned()),
            content_is_null: true,
        },
    ];
    let skipped = vec![
        skipped_record("a.bin", SKIP_REASON_UNREADABLE),
        skipped_record("b.bin", SKIP_REASON_OVERSIZE),
    ];
    let mut changes = Vec::new();

    add_classification_drift_changes(&mut changes, &stored, &[], &skipped);

    ensure_eq(&changes.len(), &1usize, "only changed skip_reason drifts")?;
    ensure_eq(
        &changes
            .first()
            .map(|c| (c.relative_path.as_str(), c.status)),
        &Some(("a.bin", ChangeStatus::Modified)),
        "a.bin marked Modified on skip_reason drift",
    )?;
    Ok(())
}

#[test]
fn filter_skipped_records_only_keeps_changed_paths_unless_rewriting_all() -> TestResult {
    let records = vec![
        skipped_record("unchanged.bin", SKIP_REASON_OVERSIZE),
        skipped_record("changed.bin", SKIP_REASON_UNREADABLE),
        skipped_record("deleted.bin", SKIP_REASON_OVERSIZE),
    ];
    let changes = vec![
        FileChange {
            relative_path: "changed.bin".to_owned(),
            status: ChangeStatus::Modified,
        },
        FileChange {
            relative_path: "deleted.bin".to_owned(),
            status: ChangeStatus::Deleted,
        },
    ];

    let incremental = filter_skipped_records(
        vec![
            skipped_record("unchanged.bin", SKIP_REASON_OVERSIZE),
            skipped_record("changed.bin", SKIP_REASON_UNREADABLE),
            skipped_record("deleted.bin", SKIP_REASON_OVERSIZE),
        ],
        &changes,
        false,
    );
    ensure_eq(
        &incremental.len(),
        &1usize,
        "only changed skipped record retained",
    )?;
    ensure_eq(
        &incremental.first().map(|r| r.relative_path.as_str()),
        &Some("changed.bin"),
        "changed.bin retained",
    )?;

    let full = filter_skipped_records(records, &changes, true);
    ensure_eq(
        &full.len(),
        &3usize,
        "rewrite_all retains every skipped record",
    )?;
    Ok(())
}
