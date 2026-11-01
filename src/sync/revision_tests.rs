use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::test_support::write;
use crate::util::now_unix;

#[test]
fn now_unix_tracks_real_wall_clock_time() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_secs();
    let value = now_unix();
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_secs();

    let value = u64::try_from(value).expect("a real current timestamp is never negative");
    assert!(
        (before..=after).contains(&value),
        "now_unix() must return the real current time, not a fixed constant: \
         expected within [{before}, {after}], got {value}"
    );
}

#[test]
fn a_first_publish_of_more_files_than_the_sqlite_variable_limit_completes() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    let file_count: i64 = 32_800;
    for i in 0..file_count {
        write(project.path(), &format!("d{}/f{i}.txt", i % 100), b"x\n");
    }

    let report = publish(&mut connection, project.path(), "1.0", &NoopProgressSink)
        .expect("a first publish touches every file and must not hit the bound-variable limit");

    let stored: i64 = connection
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .expect("count files");
    assert_eq!(stored, file_count);
    assert!(!report.revision_id.is_empty());
}

/// Every fallible step in `publish_revision` converts a `rusqlite::Error`
/// into `RevisionError::Database`; a database without the schema proves the
/// conversion and preserves the SQLite cause.
#[test]
fn a_publish_revision_database_failure_is_the_database_variant() {
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection =
        rusqlite::Connection::open(db_dir.path().join("project.db")).expect("open bare db");
    let discovered = HashSet::new();

    let error = publish_revision(
        &mut connection,
        RevisionInputs {
            expected_current: None,
            manifest_hash: "manifest",
            parser_pack_version: "1.0",
            upserts: &[],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
        },
    )
    .expect_err("a database without the schema cannot publish a revision");

    match error {
        RevisionError::Database(error) => assert!(
            error.to_string().contains("no such table: revisions"),
            "the source error must be preserved: {error}"
        ),
        other => panic!("expected RevisionError::Database, got {other:?}"),
    }
}

/// A parser run that contradicts its own invariant must fail the publish
/// with `InvalidParserRun` naming the file and the invariant's reason, not
/// be written to the database.
#[test]
fn an_invalid_parser_run_is_rejected_by_the_revision_publish() {
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
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

    let error = publish_revision(
        &mut connection,
        RevisionInputs {
            expected_current: None,
            manifest_hash: "manifest",
            parser_pack_version: "1.0",
            upserts: &[record],
            deletions: &[],
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: false,
        },
    )
    .expect_err("an invalid parser run must fail the publish");

    match error {
        RevisionError::InvalidParserRun { path, reason } => {
            assert_eq!(path, "bad.rs");
            assert_eq!(
                reason,
                "an unavailable parser cannot report a parse outcome"
            );
        }
        other => panic!("expected RevisionError::InvalidParserRun, got {other:?}"),
    }
}
