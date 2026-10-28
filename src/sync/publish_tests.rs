//! End-to-end publish scenarios: first sync, unchanged reuse, parser-version
//! reanalysis, modify+delete, invalid UTF-8, real parsed evidence, cascade
//! delete.
use super::*;
use crate::store::open_read_write;
use crate::store::test_capture::capture_warns;
use crate::sync::test_support::{stored_paths, write};
use std::fs;

#[test]
fn invalid_utf8_is_recorded_as_evidence_not_silently_swallowed() {
    let project = tempfile::tempdir().expect("project dir");
    // 0xFF is never valid as a UTF-8 lead byte.
    write(
        project.path(),
        "garbled.txt",
        &[b'a', b'b', 0xFF, b'c', b'd'],
    );
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("publish");

    // The pack may also produce its own syntax-error diagnostic for this
    // file, so check that an encoding-specific one exists among possibly
    // several Diagnostic rows, rather than assuming there's only one.
    let mut statement = connection
        .prepare(
            "SELECT json_extract(e.payload, '$.message') FROM evidence e \
             JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'garbled.txt' AND e.kind = 'Diagnostic'",
        )
        .expect("prepare");
    let messages: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert!(
        messages.iter().any(|message| message.contains("UTF-8")),
        "expected an encoding diagnostic among {messages:?}"
    );

    let content: String = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'garbled.txt'",
            [],
            |row| row.get(0),
        )
        .expect("read content");
    assert!(
        content.contains('\u{FFFD}'),
        "content should contain the replacement character"
    );
}

#[test]
fn first_sync_publishes_every_discovered_file() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "src/main.rs", b"fn main() {}");
    write(project.path(), "README.md", b"# Title");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    let report = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("publish");
    assert_eq!(report.added, 2);
    assert_eq!(report.modified, 0);
    assert_eq!(report.deleted, 0);
    assert_eq!(stored_paths(&connection), vec!["README.md", "src/main.rs"]);
}

#[test]
fn unchanged_sync_reuses_the_current_revision_and_touches_nothing() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "src/main.rs", b"fn main() {}");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    let first = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("first publish");
    let second = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("second publish");

    assert_eq!(first.revision_id, second.revision_id);
    assert_eq!(second.added, 0);
    assert_eq!(second.modified, 0);
    assert_eq!(second.unchanged, 1);
}

#[test]
fn a_parser_version_change_forces_reanalysis_despite_no_file_changes() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"pub fn a() {}\n");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    let first = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("first publish");
    // Nothing on disk changes, only the parser version does.
    let second = publish(
        &mut connection,
        project.path(),
        "2.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("second publish");

    assert_ne!(
        first.revision_id, second.revision_id,
        "a parser version change must publish a new revision even with an unchanged file set"
    );

    let stored_version: String = connection
        .query_row(
            "SELECT parser_pack_version FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .expect("read stored parser version");
    assert_eq!(stored_version, "2.0.0");
}

#[test]
fn modified_file_replaces_its_row_and_deleted_file_is_purged() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "src/main.rs", b"fn main() {}");
    write(project.path(), "src/lib.rs", b"pub fn lib() {}");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("first publish");

    write(project.path(), "src/main.rs", b"fn main() { changed(); }");
    fs::remove_file(project.path().join("src/lib.rs")).expect("remove file");

    let report = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("second publish");
    assert_eq!(report.modified, 1);
    assert_eq!(report.deleted, 1);
    assert_eq!(stored_paths(&connection), vec!["src/main.rs"]);

    let content: String = connection
        .query_row(
            "SELECT content FROM files WHERE path = 'src/main.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read content");
    assert_eq!(content, "fn main() { changed(); }");
}

#[test]
fn a_real_rust_file_gets_real_parsed_evidence_not_a_placeholder() {
    let project = tempfile::tempdir().expect("project dir");
    write(
        project.path(),
        "src/lib.rs",
        b"pub fn greet() {\n    println!(\"hi\");\n}\n",
    );
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("publish");

    let (language, availability, outcome, completeness): (String, String, String, String) =
        connection
            .query_row(
                "SELECT language, parser_availability, parse_outcome, extraction_completeness \
             FROM files WHERE path = 'src/lib.rs'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read file status");
    assert_eq!(language, "rust");
    assert_eq!(availability, "Available");
    assert_eq!(outcome, "Succeeded");
    assert_eq!(completeness, "Full", "a clean parse degraded nothing");

    let structure_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'src/lib.rs' AND e.kind = 'Structure'",
            [],
            |row| row.get(0),
        )
        .expect("count structure evidence");
    assert!(
        structure_count >= 1,
        "expected at least the greet() function"
    );
}

#[test]
fn deleting_a_file_cascades_its_evidence() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "src/main.rs", b"fn main() {}");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("first publish");

    let file_id: i64 = connection
        .query_row(
            "SELECT id FROM files WHERE path = 'src/main.rs'",
            [],
            |row| row.get(0),
        )
        .expect("read file id");
    connection
        .execute(
            "INSERT INTO evidence (file_id, key, kind, origin, span_availability, payload) \
             VALUES (?1, 'main:0', 'Symbol', 'PackSymbol', 'PackOmitted', '{}')",
            [file_id],
        )
        .expect("insert evidence");

    fs::remove_file(project.path().join("src/main.rs")).expect("remove file");
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("second publish");

    let remaining_evidence: i64 = connection
        .query_row("SELECT count(*) FROM evidence", [], |row| row.get(0))
        .expect("count evidence");
    assert_eq!(remaining_evidence, 0);
}

/// Both public entry points reject a parser pack version that is not a
/// non-empty numeric dotted version before discovery, so a malformed pin
/// can never reach the revision row.
#[test]
fn an_invalid_parser_pack_version_is_rejected_by_both_publish_entry_points() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    let via_limits = publish_with_limits(
        &mut connection,
        project.path(),
        "1.x",
        &crate::progress::NoopProgressSink,
        &ResourceLimits::default(),
    )
    .expect_err("a non-numeric version must be rejected");
    assert_eq!(
        via_limits.to_string(),
        "invalid parser pack version \"1.x\"; expected a non-empty numeric dotted version"
    );

    let via_public = publish(
        &mut connection,
        project.path(),
        "",
        &crate::progress::NoopProgressSink,
    )
    .expect_err("an empty version must be rejected");
    match via_public {
        PublishError::InvalidParserPackVersion(version) => assert_eq!(version, ""),
        other => panic!("expected InvalidParserPackVersion, got {other:?}"),
    }
}

/// The baseline revision read is the first database operation of a publish;
/// its `rusqlite::Error` must surface as the exact `PublishError::Database`
/// variant with the SQLite cause preserved, not a generic failure.
#[test]
fn a_publish_whose_baseline_read_fails_is_the_database_variant() {
    let project = tempfile::tempdir().expect("project dir");
    write(project.path(), "lib.rs", b"fn a() {}\n");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection =
        rusqlite::Connection::open(db_dir.path().join("project.db")).expect("open bare db");

    let error = publish_with_limits(
        &mut connection,
        project.path(),
        "1.0",
        &crate::progress::NoopProgressSink,
        &ResourceLimits::default(),
    )
    .expect_err("a database without the schema cannot read the baseline revision");

    match error {
        PublishError::Database(error) => assert!(
            error.to_string().contains("no such table: revisions"),
            "the source error must be preserved: {error}"
        ),
        other => panic!("expected PublishError::Database, got {other:?}"),
    }
}

/// A sampling skip's report reason is `SampleError::TooLarge`'s Display,
/// which embeds the absolute path. The warn line must correlate the file
/// through the redacted path id and the static reason kind only.
#[test]
fn a_sampling_skip_logs_the_redacted_path_not_the_absolute_skip_message() {
    let project = tempfile::tempdir().expect("project dir");
    let huge = project.path().join("huge.rs");
    std::fs::write(&huge, vec![b'x'; 100_000]).expect("write oversized file");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");
    let limits = ResourceLimits {
        max_file_bytes: 50_000,
        ..ResourceLimits::default()
    };

    let (result, logs) = capture_warns(|| {
        publish_with_limits(
            &mut connection,
            project.path(),
            "1.0",
            &crate::progress::NoopProgressSink,
            &limits,
        )
    });

    result.expect("an oversized file is skipped, not fatal");
    let raw = huge.to_string_lossy();
    assert!(
        !logs.contains(raw.as_ref()),
        "the absolute path must not be logged: {logs}"
    );
    assert!(
        logs.contains(&crate::util::redacted_path(&huge)),
        "the redacted path id must identify the skipped file: {logs}"
    );
    assert!(
        logs.contains("reason_kind=\"oversize\""),
        "the static reason must be logged: {logs}"
    );
}

/// A failed publish logs its category, the real path and the error's
/// `Display`, so the OS cause (`gone`) and the file are in the log.
#[test]
fn honest_log_a_failed_publish_logs_the_real_path_and_io_cause() {
    let root = std::path::Path::new("/tmp/once-probe-project");
    let absolute = std::path::Path::new("/home/private-user/secret-repo/src/lib.rs");
    let result: Result<PublishReport, PublishError> =
        Err(PublishError::Sample(sample::SampleError::Read {
            path: absolute.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        }));

    let (_, logs) = capture_warns(|| crate::sync::publish_log::log_outcome(root, &result, 0));

    assert!(logs.contains("publish failed"), "got: {logs}");
    assert!(logs.contains("error_kind=\"sample_read\""), "got: {logs}");
    assert!(
        logs.contains("path=/home/private-user/secret-repo/src/lib.rs"),
        "the real path must identify the file: {logs}"
    );
    assert!(
        logs.contains("gone"),
        "the underlying io cause must be logged: {logs}"
    );
    assert_eq!(
        logs.matches("publish failed").count(),
        1,
        "one failure is one record: {logs}"
    );
}

/// A SQLite failure has no path; its message is the only diagnosis.
#[test]
fn honest_log_a_failed_publish_logs_the_sqlite_cause() {
    let root = std::path::Path::new("/tmp/once-probe-project");
    let result: Result<PublishReport, PublishError> =
        Err(PublishError::Database(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("SENTINEL_SQLITE_CAUSE".to_owned()),
        )));

    let (_, logs) = capture_warns(|| crate::sync::publish_log::log_outcome(root, &result, 0));

    assert!(logs.contains("error_kind=\"database\""), "got: {logs}");
    assert!(
        logs.contains("SENTINEL_SQLITE_CAUSE"),
        "the SQLite message must be logged: {logs}"
    );
    assert!(logs.contains("root=/tmp/once-probe-project"), "got: {logs}");
}

/// Skip collection now lives beside the diff that consumes it; these tests
/// stay with the publish scenarios that own the "logged once" contract.
mod publish_attempt_skip_logging {
    use crate::store::test_capture::capture_warns;
    use crate::sync::discovery;
    use crate::sync::publish_diff::collect_skipped;
    use crate::sync::revision_skipped::SKIP_REASON_STAT_FAILED;
    use std::collections::{BTreeMap, HashSet};

    fn skipped(path: std::path::PathBuf) -> discovery::SkippedFile {
        discovery::SkippedFile {
            absolute_path: path,
            reason: "read failed".to_owned(),
        }
    }

    #[test]
    fn a_stat_failed_skip_logs_exactly_one_warn_line() {
        let root = tempfile::tempdir().expect("tempdir");
        let files = [skipped(root.path().join("vanished.rs"))];
        let mut paths = HashSet::new();
        let mut records = BTreeMap::new();

        let (_, logs) = capture_warns(|| {
            collect_skipped(
                root.path(),
                &files,
                "once-probe-stat-failed",
                &mut paths,
                &mut records,
            );
        });

        let lines = logs
            .lines()
            .filter(|line| line.contains("once-probe-stat-failed"))
            .count();
        assert_eq!(lines, 1, "one warn per stat-failed skip: {logs}");
        assert_eq!(records["vanished.rs"].reason, SKIP_REASON_STAT_FAILED);
        assert_eq!(records["vanished.rs"].byte_len, None);
    }

    #[test]
    fn a_skip_whose_stat_succeeds_keeps_the_publish_loop_warn() {
        let root = tempfile::tempdir().expect("tempdir");
        let present = root.path().join("present.rs");
        std::fs::write(&present, b"fn a() {}").expect("write file");
        let files = [skipped(present)];
        let mut paths = HashSet::new();
        let mut records = BTreeMap::new();

        let (_, logs) = capture_warns(|| {
            collect_skipped(
                root.path(),
                &files,
                "once-probe-stat-ok",
                &mut paths,
                &mut records,
            );
        });

        let lines = logs
            .lines()
            .filter(|line| line.contains("once-probe-stat-ok"))
            .count();
        assert_eq!(lines, 1, "{logs}");
        assert_eq!(records["present.rs"].reason, "once-probe-stat-ok");
    }
}
