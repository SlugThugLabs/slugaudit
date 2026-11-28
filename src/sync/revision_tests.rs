use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::test_support::write;
use crate::util::now_unix;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn now_unix_tracks_real_wall_clock_time() -> TestResult {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .check("system clock is after the epoch")?
        .as_secs();
    let value = now_unix().check("read the clock")?;
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .check("system clock is after the epoch")?
        .as_secs();

    let value = u64::try_from(value).check("a real current timestamp is never negative")?;
    ensure(
        (before..=after).contains(&value),
        format!(
            "now_unix() must return the real current time, not a fixed constant: \
         expected within [{before}, {after}], got {value}"
        ),
    )?;
    Ok(())
}

#[test]
fn a_first_publish_of_more_files_than_the_sqlite_variable_limit_completes() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let file_count: i64 = 32_800;
    for i in 0..file_count {
        write(project.path(), &format!("d{}/f{i}.txt", i % 100), b"x\n")?;
    }

    let report = publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
        .check("a first publish touches every file and must not hit the bound-variable limit")?;

    let stored: i64 = connection
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .check("count files")?;
    ensure_eq(&stored, &file_count, "stored == file_count")?;
    ensure(
        !report.revision_id.is_empty(),
        "!report.revision_id.is_empty()",
    )?;
    Ok(())
}

/// Every fallible step in `publish_revision` converts a `rusqlite::Error`
/// into `RevisionError::Database`; a database without the schema proves the
/// conversion and preserves the SQLite cause.
#[test]
fn a_publish_revision_database_failure_is_the_database_variant() -> TestResult {
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection =
        rusqlite::Connection::open(db_dir.path().join("project.db")).check("open bare db")?;
    let discovered = HashSet::new();
    let unrepresentable = HashSet::new();

    let error = publish_revision(
        &mut connection,
        &RevisionInputs {
            expected_current: None,
            parser_pack_version: "1.0",
            upserts: &[],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
            unrepresentable_paths: &unrepresentable,
            deadline: None,
        },
    )
    .check_err("a database without the schema cannot publish a revision")?;

    match error {
        RevisionError::Database(error) => ensure(
            error.to_string().contains("no such table: revisions"),
            format!("the source error must be preserved: {error}"),
        )?,
        other => {
            return Err(format!("expected RevisionError::Database, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}

/// A parser run that contradicts its own invariant must fail the publish
/// with `InvalidParserRun` naming the file and the invariant's reason, not
/// be written to the database.
#[test]
fn an_invalid_parser_run_is_rejected_by_the_revision_publish() -> TestResult {
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let record = FileRecord {
        relative_path: "bad.rs".to_owned(),
        is_binary: false,
        content: Some("fn a() {}\n".to_owned()),
        identity: SourceIdentity {
            relative_path: "bad.rs".to_owned(),
            content_hash: "hash".to_owned(),
            hash_algorithm: "blake3".to_owned(),
        },
        byte_len: 10,
        modified_unix_seconds: None,
        language: Some("rust".to_owned()),
        language_detected: true,
        run: ParserRun {
            // Unavailable requires NotAttempted, so this run is invalid.
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::Succeeded,
            completeness: ExtractionCompleteness::Full,
        },
        evidence: Vec::new(),
    };
    let discovered = HashSet::new();
    let unrepresentable = HashSet::new();

    let error = publish_revision(
        &mut connection,
        &RevisionInputs {
            expected_current: None,
            parser_pack_version: "1.0",
            upserts: &[record],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
            unrepresentable_paths: &unrepresentable,
            deadline: None,
        },
    )
    .check_err("an invalid parser run must fail the publish")?;

    match error {
        RevisionError::InvalidParserRun { path, reason } => {
            ensure_eq(&path, &"bad.rs", "path == \"bad.rs\"")?;
            ensure_eq(
                &reason,
                &"an unavailable parser cannot report a parse outcome",
                "reason == \"an unavailable parser cannot report a parse outcome\"",
            )?;
        }
        other => {
            return Err(format!(
                "expected RevisionError::InvalidParserRun, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn a_baseline_label_is_the_revision_id_or_none() -> TestResult {
    ensure_eq(
        &baseline_label(Some("rev-1")),
        &"rev-1",
        "baseline_label(Some(\"rev-1\")) == \"rev-1\"",
    )?;
    ensure_eq(
        &baseline_label(None),
        &"none",
        "baseline_label(None) == \"none\"",
    )?;
    Ok(())
}

#[test]
fn publish_revision_fails_closed_when_deadline_is_spent() -> TestResult {
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let discovered = HashSet::new();
    let unrepresentable = HashSet::new();
    let expired = crate::util::Deadline::after(std::time::Duration::ZERO);

    let error = publish_revision(
        &mut connection,
        &RevisionInputs {
            expected_current: None,
            parser_pack_version: "1.0",
            upserts: &[],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
            unrepresentable_paths: &unrepresentable,
            deadline: Some(&expired),
        },
    )
    .check_err("spent deadline must abort revision publish")?;

    ensure(
        matches!(error, RevisionError::TimeBudgetExceeded { .. }),
        format!("expected TimeBudgetExceeded, got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn stale_baseline_leaves_unrepresentable_paths_unchanged() -> TestResult {
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let discovered = HashSet::new();
    let initial_unrep = HashSet::from([b"bad-\xff.rs".to_vec()]);

    let rev1 = publish_revision(
        &mut connection,
        &RevisionInputs {
            expected_current: None,
            parser_pack_version: "1.0",
            upserts: &[],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
            unrepresentable_paths: &initial_unrep,
            deadline: None,
        },
    )
    .check("first revision")?;
    ensure_eq(&rev1, &"rev-1", "first revision id")?;

    // A stale CAS attempt with different unrepresentable_paths must fail
    // without mutating unrepresentable_paths.
    let stale_unrep = HashSet::from([b"other-\xfe.rs".to_vec()]);
    let error = publish_revision(
        &mut connection,
        &RevisionInputs {
            expected_current: None,
            parser_pack_version: "1.0",
            upserts: &[],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
            unrepresentable_paths: &stale_unrep,
            deadline: None,
        },
    )
    .check_err("stale baseline must fail")?;
    ensure(
        matches!(error, RevisionError::StaleBaseline { .. }),
        format!("expected StaleBaseline, got {error:?}"),
    )?;

    let stored = crate::sync::publish::stored_unrepresentable_paths(&connection)
        .check("read stored unrepresentable_paths")?;
    ensure_eq(&stored, &initial_unrep, "unrepresentable_paths unchanged")?;
    Ok(())
}

#[test]
fn apply_deletions_and_upsert_skipped_files_respect_spent_deadline() -> TestResult {
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let expired = crate::util::Deadline::after(std::time::Duration::ZERO);
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .check("begin tx")?;

    let del_err = apply_deletions(&tx, &["gone.rs".to_owned()], Some(&expired))
        .check_err("apply_deletions must fail on spent deadline")?;
    ensure(
        matches!(del_err, RevisionError::TimeBudgetExceeded { .. }),
        format!("expected TimeBudgetExceeded from apply_deletions, got {del_err:?}"),
    )?;

    let skipped = vec![SkippedRecord {
        relative_path: "big.bin".to_owned(),
        reason: SKIP_REASON_OVERSIZE.to_owned(),
        identity: SourceIdentity::new("big.bin".to_owned(), "stat-h".to_owned()),
        byte_len: Some(999),
        modified_unix_seconds: None,
    }];
    let skip_err = upsert_skipped_files(&tx, &skipped, 1, Some(&expired))
        .check_err("upsert_skipped_files must fail on spent deadline")?;
    ensure(
        matches!(skip_err, RevisionError::TimeBudgetExceeded { .. }),
        format!("expected TimeBudgetExceeded from upsert_skipped_files, got {skip_err:?}"),
    )?;
    Ok(())
}
