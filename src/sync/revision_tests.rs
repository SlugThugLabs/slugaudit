use super::*;
use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability};
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::test_support::write;
use crate::util::{Deadline, now_unix};
use std::time::Duration;

/// A cap-busting path count: SQLite's bound-variable limit is 32766.
const OVER_VARIABLE_LIMIT: usize = 40_000;

fn insert_finding(connection: &Connection, path: &str, source_hash: &str) {
    connection
        .execute(
            "INSERT INTO findings (path, source_hash, line_start, line_end, severity, \
             category, title, description, created_at_unix, evidence_revision) \
             VALUES (?1, ?2, 1, 1, 's', 'c', 't', 'd', 0, 'r')",
            params![path, source_hash],
        )
        .expect("insert finding");
}

fn finding_statuses(connection: &Connection) -> Vec<(String, String)> {
    connection
        .prepare("SELECT path, status FROM findings ORDER BY path")
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

#[test]
fn stale_findings_are_invalidated_when_touched_paths_exceed_the_sqlite_variable_limit() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    write(project.path(), "same.rs", b"fn same() {}\n");
    write(project.path(), "changed.rs", b"fn changed() {}\n");
    write(project.path(), "untouched.rs", b"fn untouched() {}\n");
    publish(&mut connection, project.path(), "1.0", &NoopProgressSink).expect("seed publish");
    let same_hash: String = connection
        .query_row(
            "SELECT content_hash FROM files WHERE path = 'same.rs'",
            [],
            |row| row.get(0),
        )
        .expect("stored hash");
    insert_finding(&connection, "same.rs", &same_hash);
    insert_finding(&connection, "changed.rs", "outdated-hash");
    insert_finding(&connection, "untouched.rs", "outdated-hash");
    insert_finding(&connection, "gone.rs", "any-hash");
    let mut touched: Vec<String> = (0..OVER_VARIABLE_LIMIT)
        .map(|i| format!("filler/{i}.rs"))
        .collect();
    touched.extend(["same.rs", "changed.rs", "gone.rs"].map(str::to_owned));
    let touched: Vec<&str> = touched.iter().map(String::as_str).collect();
    let tx = connection.transaction().expect("transaction");

    invalidate_stale_findings(&tx, &touched).expect("invalidate with 40k touched paths");
    tx.commit().expect("commit");

    let expected = [
        ("changed.rs", "stale"),
        ("gone.rs", "stale"),
        ("same.rs", "current"),
        ("untouched.rs", "current"),
    ]
    .map(|(path, status)| (path.to_owned(), status.to_owned()));
    assert_eq!(finding_statuses(&connection), expected);
}

/// Batching must not change which findings go stale: stale findings in
/// different chunks and a still-matching finding in the last chunk prove the
/// predicate runs for every batch, not only the first or last.
#[test]
fn stale_findings_are_invalidated_across_every_path_batch() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    write(project.path(), "same.rs", b"fn same() {}\n");
    write(project.path(), "changed.rs", b"fn changed() {}\n");
    write(project.path(), "untouched.rs", b"fn untouched() {}\n");
    publish(&mut connection, project.path(), "1.0", &NoopProgressSink).expect("seed publish");
    let same_hash: String = connection
        .query_row(
            "SELECT content_hash FROM files WHERE path = 'same.rs'",
            [],
            |row| row.get(0),
        )
        .expect("stored hash");
    insert_finding(&connection, "same.rs", &same_hash);
    insert_finding(&connection, "changed.rs", "outdated-hash");
    insert_finding(&connection, "untouched.rs", "outdated-hash");
    insert_finding(&connection, "gone.rs", "any-hash");
    // Five touched paths through batches of two: the stale findings land in
    // different chunks and the still-matching one shares the last chunk.
    let touched = [
        "filler/first.rs",
        "changed.rs",
        "filler/second.rs",
        "gone.rs",
        "same.rs",
    ];
    let tx = connection.transaction().expect("transaction");

    invalidate_stale_findings_in_batches(&tx, &touched, 2).expect("invalidate in batches of two");
    tx.commit().expect("commit");

    let expected = [
        ("changed.rs", "stale"),
        ("gone.rs", "stale"),
        ("same.rs", "current"),
        ("untouched.rs", "current"),
    ]
    .map(|(path, status)| (path.to_owned(), status.to_owned()));
    assert_eq!(finding_statuses(&connection), expected);
}

#[test]
fn the_manifest_hash_excludes_every_path_of_a_set_beyond_the_sqlite_variable_limit() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    write(project.path(), "a.rs", b"fn a() {}\n");
    write(project.path(), "b.rs", b"fn b() {}\n");
    publish(&mut connection, project.path(), "1.0", &NoopProgressSink).expect("seed publish");
    let mut deletions: Vec<String> = (0..OVER_VARIABLE_LIMIT)
        .map(|i| format!("filler/{i}.rs"))
        .collect();
    let deadline = Deadline::after(Duration::from_secs(60));
    let without_deleting_a =
        crate::sync::manifest::compute_manifest_hash(&connection, &[], &deletions, &[], &deadline)
            .expect("hash with 40k unrelated deletions");
    deletions.push("a.rs".to_owned());

    let with_a_deleted =
        crate::sync::manifest::compute_manifest_hash(&connection, &[], &deletions, &[], &deadline)
            .expect("hash with 40k+1 deletions");

    assert_ne!(
        without_deleting_a, with_a_deleted,
        "a deleted stored path must leave the manifest even in a huge exclusion set"
    );
}

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
