//! One realistic, multi-language project pushed through the real
//! `sync::publish` pipeline end to end — discovery, sampling, per-language
//! parsing, evidence normalization, dependency-edge resolution, and the
//! atomic write — rather than any single stage in isolation. Not a stress
//! test: a handful of files chosen to each exercise one documented behavior
//! (malformed source, a binary file, a circular import pair, a deletion, and
//! a "rename" simulated as delete+add) at once, the way a small real
//! polyglot repository actually looks.
use super::*;
use crate::store::open_read_write;
use crate::sync::test_support::{stored_paths, write};
use std::fs;

struct FileRow {
    file_kind: String,
    content: Option<String>,
    language: Option<String>,
    parse_outcome: String,
}

fn file_row(connection: &Connection, path: &str) -> FileRow {
    connection
        .query_row(
            "SELECT file_kind, content, language, parse_outcome FROM files WHERE path = ?1",
            [path],
            |row| {
                Ok(FileRow {
                    file_kind: row.get(0)?,
                    content: row.get(1)?,
                    language: row.get(2)?,
                    parse_outcome: row.get(3)?,
                })
            },
        )
        .unwrap_or_else(|error| panic!("expected a row for {path}: {error}"))
}

fn diagnostic_messages(connection: &Connection, path: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT json_extract(e.payload, '$.message') FROM evidence e \
             JOIN files f ON f.id = e.file_id \
             WHERE f.path = ?1 AND e.kind = 'Diagnostic'",
        )
        .expect("prepare");
    statement
        .query_map([path], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

fn resolved_target(connection: &Connection, from: &str) -> Option<String> {
    connection
        .query_row(
            "SELECT t.path FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             LEFT JOIN files t ON t.id = e.to_file_id \
             WHERE f.path = ?1 AND e.resolution_kind = 'Resolved'",
            [from],
            |row| row.get(0),
        )
        .ok()
}

#[test]
fn a_realistic_polyglot_project_syncs_correctly_end_to_end() {
    let project = tempfile::tempdir().expect("project dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open db");

    // Python package with a real cross-module import.
    write(project.path(), "pkg/__init__.py", b"");
    write(project.path(), "pkg/helper.py", b"def f():\n    pass\n");
    write(project.path(), "pkg/main.py", b"from .helper import f\n");
    // Genuinely malformed Python: an unclosed parameter list.
    write(project.path(), "pkg/broken.py", b"def broken(:\n    pass\n");
    // Valid Rust.
    write(project.path(), "src/lib.rs", b"pub fn greet() {}\n");
    // A circular JavaScript import pair.
    write(
        project.path(),
        "src/circular_a.js",
        b"import { b } from './circular_b.js';\nexport function a() {}\n",
    );
    write(
        project.path(),
        "src/circular_b.js",
        b"import { a } from './circular_a.js';\nexport function b() {}\n",
    );
    // A binary file, sniffed via its NUL byte.
    write(
        project.path(),
        "assets/logo.png",
        &[0x89, b'P', b'N', b'G', 0x00, 0x01, 0x02],
    );
    // Will be deleted outright on the next sync.
    write(project.path(), "to_delete.py", b"x = 1\n");
    // Will be "renamed" (delete + add with identical content) on the next sync.
    write(project.path(), "old_name.py", b"y = 2\n");

    let first = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("first publish");
    assert_eq!(first.added, 10);
    assert_eq!(first.deleted, 0);

    // Malformed source is recorded as evidence, not silently treated as a
    // clean parse.
    let broken = file_row(&connection, "pkg/broken.py");
    assert_eq!(broken.parse_outcome, "SyntaxErrors");
    assert!(
        !diagnostic_messages(&connection, "pkg/broken.py").is_empty(),
        "a malformed file must produce at least one Diagnostic evidence row"
    );

    // The binary file is classified and inert: no content, no language, and
    // (implicitly, via foreign key cascade behavior elsewhere) no evidence.
    let logo = file_row(&connection, "assets/logo.png");
    assert_eq!(logo.file_kind, "binary");
    assert_eq!(logo.content, None);
    assert_eq!(logo.language, None);
    let logo_evidence: i64 = connection
        .query_row(
            "SELECT count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
             WHERE f.path = 'assets/logo.png'",
            [],
            |row| row.get(0),
        )
        .expect("count logo evidence");
    assert_eq!(logo_evidence, 0, "a binary file must carry no evidence");

    // The real Python cross-module import resolves.
    assert_eq!(
        resolved_target(&connection, "pkg/main.py").as_deref(),
        Some("pkg/helper.py")
    );

    // Both directions of the circular JS import pair resolve independently;
    // nothing loops or panics resolving a cycle.
    assert_eq!(
        resolved_target(&connection, "src/circular_a.js").as_deref(),
        Some("src/circular_b.js")
    );
    assert_eq!(
        resolved_target(&connection, "src/circular_b.js").as_deref(),
        Some("src/circular_a.js")
    );

    // --- Second sync: a deletion, and a rename simulated as delete+add. ---
    fs::remove_file(project.path().join("to_delete.py")).expect("delete file");
    fs::remove_file(project.path().join("old_name.py")).expect("remove old name");
    write(project.path(), "new_name.py", b"y = 2\n");

    let second = publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("second publish");
    assert_eq!(
        second.deleted, 2,
        "the outright deletion and the old half of the rename both count as deletions"
    );
    assert_eq!(
        second.added, 1,
        "this codebase does not track renames as a distinct operation: \
         the new half of the rename is just an Added file with the same content"
    );

    let paths = stored_paths(&connection);
    assert!(!paths.contains(&"to_delete.py".to_owned()));
    assert!(!paths.contains(&"old_name.py".to_owned()));
    assert!(paths.contains(&"new_name.py".to_owned()));
    let renamed = file_row(&connection, "new_name.py");
    assert_eq!(renamed.content.as_deref(), Some("y = 2\n"));

    // Everything untouched by the second sync is still intact.
    assert_eq!(
        resolved_target(&connection, "src/circular_a.js").as_deref(),
        Some("src/circular_b.js")
    );
    let logo_after: FileRow = file_row(&connection, "assets/logo.png");
    assert_eq!(logo_after.file_kind, "binary");
}
