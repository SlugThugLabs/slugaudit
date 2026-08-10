//! End-to-end publish scenarios: first sync, unchanged reuse, parser-version
//! reanalysis, modify+delete, invalid UTF-8, real parsed evidence, cascade
//! delete.
// slugaudit-line-exception: approved-by=agent; reason=one end-to-end publish scenario per sync invariant, all sharing the write/stored_paths fixture helpers against a real SQLite database; splitting would force a cross-module test harness or duplicate the helpers in every file
use super::*;
use crate::store::open_read_write;
use std::fs;

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

fn stored_paths(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .expect("prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

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
    assert_eq!(completeness, "Partial");

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
