use super::{
    StoredRow, add_available_grammar_retries, add_classification_drift_changes,
    add_load_failed_retries, diff_against_stored, filter_skipped_records,
    unavailable_language_paths,
};
use crate::model::SourceIdentity;
use crate::parse::fetch_gate::test_controls::GateResetGuard;
use crate::sync::manifest::{ChangeStatus, FileChange};
use crate::sync::revision::{SKIP_REASON_OVERSIZE, SKIP_REASON_UNREADABLE, SkippedRecord};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
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
    let _gate_guard = GateResetGuard::new();
    let _failures_guard = crate::parse::local_load_failures::LocalLoadFailuresGuard::clear();
    crate::parse::local_load_failures::record_local_load_failure("rust")
        .check("record local failure for rust")?;
    let load_failed_map = HashMap::from([
        ("a.rs".to_owned(), Some("rust".to_owned())),
        ("b.py".to_owned(), Some("python".to_owned())),
    ]);
    let current = vec![("a.rs", "h1"), ("b.py", "h2")];
    let mut changes = Vec::new();

    add_load_failed_retries(&mut changes, &current, &load_failed_map)
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

#[test]
fn oversize_for_parse_file_is_not_retried_by_unavailable_language_paths_or_warm_sync() -> TestResult
{
    let _gate_guard = GateResetGuard::new();
    let _failures_guard = crate::parse::local_load_failures::LocalLoadFailuresGuard::clear();
    let _config = crate::tools::test_support::use_global_config(r#"{"max_parse_bytes": 16}"#)?;
    let project = crate::sync::test_support::create_project()?;
    crate::sync::test_support::write_file(
        &project,
        "big.rs",
        b"pub fn oversized_for_parse() -> usize { 42 }\n",
    )?;
    let manager = crate::sync::SourceSyncManager::new();

    let first_sync = crate::sync::test_support::sync_project(&manager, &project)?;
    let mut connection =
        crate::store::open_read_write(&first_sync.database_path).check("open project.db")?;
    let row: (String, String, Option<String>) = connection
        .query_row(
            "SELECT parser_availability, parse_outcome, parse_error_reason \
             FROM files WHERE path = 'big.rs'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .check("query big.rs parser columns")?;
    ensure_eq(
        &row,
        &(
            "Unavailable".to_owned(),
            "NotAttempted".to_owned(),
            Some("oversize_for_parse".to_owned()),
        ),
        "big.rs is stored as Unavailable / NotAttempted / oversize_for_parse",
    )?;

    let unavailable =
        unavailable_language_paths(&connection).check("query unavailable_language_paths")?;
    ensure(
        !unavailable.contains_key("big.rs"),
        format!("oversize_for_parse file must not be returned for retry: {unavailable:?}"),
    )?;

    let second_publish = crate::sync::publish::publish(
        &mut connection,
        project.path(),
        crate::parse::PACK_VERSION,
        &crate::progress::NoopProgressSink,
    )
    .check("second warm publish")?;
    ensure_eq(
        &second_publish.modified,
        &0usize,
        "second warm publish marks 0 files modified",
    )?;
    ensure_eq(
        &second_publish.added,
        &0usize,
        "second warm publish marks 0 files added",
    )?;
    ensure_eq(
        &second_publish.deleted,
        &0usize,
        "second warm publish marks 0 files deleted",
    )?;
    ensure_eq(
        &second_publish.revision_id,
        &first_sync.revision_id,
        "second warm publish keeps the same revision_id",
    )?;

    drop(connection);
    let second_sync = crate::sync::test_support::sync_project(&manager, &project)?;
    ensure_eq(
        &second_sync.revision_id,
        &first_sync.revision_id,
        "second warm ensure_current keeps the same revision_id",
    )?;
    Ok(())
}

#[test]
fn pack_unknown_unavailable_language_is_not_retried_when_gate_is_open() -> TestResult {
    let _gate_guard = GateResetGuard::new();
    let _failures_guard = crate::parse::local_load_failures::LocalLoadFailuresGuard::clear();
    ensure(
        !crate::parse::language_available("slugaudit_nonexistent_lang"),
        "test language must not be known to the pack",
    )?;
    ensure(
        crate::parse::fetch_gate::load_failed_retry_due().check("read fetch gate")?,
        "fetch gate starts open",
    )?;

    let project = crate::sync::test_support::create_project()?;
    let bytes = b"plain unknown-language text\n";
    crate::sync::test_support::write_file(&project, "notes.unknown", bytes)?;
    let manager = crate::sync::SourceSyncManager::new();
    let first_sync = crate::sync::test_support::sync_project(&manager, &project)?;

    let connection =
        crate::store::open_read_write(&first_sync.database_path).check("open project.db")?;
    connection
        .execute(
            "UPDATE files \
             SET language = 'slugaudit_nonexistent_lang', \
                 parser_availability = 'Unavailable', \
                 parse_outcome = 'NotAttempted', \
                 parse_error_reason = NULL \
             WHERE path = 'notes.unknown'",
            [],
        )
        .check("set pack-unknown language on stored row")?;

    let unavailable =
        unavailable_language_paths(&connection).check("query unavailable_language_paths")?;
    ensure_eq(
        &unavailable.get("notes.unknown").map(String::as_str),
        &Some("slugaudit_nonexistent_lang"),
        "stored row enters unavailable candidate map",
    )?;

    let identity = crate::sync::hash_bytes("notes.unknown", bytes);
    let current = vec![("notes.unknown", identity.content_hash.as_str())];
    let mut changes = Vec::new();
    add_available_grammar_retries(&mut changes, &current, &unavailable)
        .check("add available grammar retries")?;
    ensure(
        changes.is_empty(),
        format!("pack-unknown language must not be retried: {changes:?}"),
    )?;

    let discovered = vec![crate::sync::DiscoveredFile {
        relative_path: "notes.unknown".to_owned(),
        absolute_path: project.path().join("notes.unknown"),
        kind: crate::sync::FileKind::Indexed,
    }];
    let diff = diff_against_stored(
        &connection,
        &discovered,
        &[identity],
        &[],
        &crate::sync::test_support::generous_deadline(),
    )
    .check("diff against stored")?;
    ensure(
        diff.changes.is_empty(),
        format!(
            "diff_against_stored must emit 0 retries: {:?}",
            diff.changes
        ),
    )?;
    Ok(())
}
