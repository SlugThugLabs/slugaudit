//! Regression test for binary classification during incremental
//! reconcile. Split out from `reconcile_tests.rs` so that file stays
//! under the source-size hard cap.

use super::*;
use crate::sync::publish::publish;
use crate::sync::test_support::{setup_project, write};

/// A binary file that changes on disk must stay classified as binary —
/// `content` stays NULL and `file_kind` stays 'binary', matching what a
/// fresh import would have stored. Regression test for the bug where
/// `reconcile_dirty_paths` hardcoded `FileKind::Indexed` and re-indexed a
/// modified binary as lossy UTF-8 text.
#[test]
fn modified_binary_dirty_path_stays_binary_excluded() {
    let (project, _db_dir, mut connection, _revision) = setup_project();

    // Add a binary file (NUL bytes) and publish — the initial import
    // sniffs it as binary and stores it without content.
    write(
        project.path(),
        "logo.bin",
        b"\x89PNG\x00\x01\x02\x00\x00binary-v1",
    );
    let report = publish(
        &mut connection,
        project.path(),
        "1.0",
        &crate::progress::NoopProgressSink,
    )
    .expect("publish with binary file");
    let row: (String, Option<String>) = connection
        .query_row(
            "SELECT file_kind, content FROM files WHERE path = 'logo.bin'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read binary row");
    assert_eq!(row, ("binary".to_owned(), None));

    // Modify the binary file so its hash differs, then reconcile it as a
    // dirty path — exactly what the watcher path does.
    write(
        project.path(),
        "logo.bin",
        b"\x89PNG\x00\x01\x02\x00\x00binary-v2",
    );
    let dirty = ["logo.bin"].iter().map(|s| s.to_string()).collect();
    let deleted = HashSet::new();
    let report = reconcile_dirty_paths(
        &mut connection,
        project.path(),
        dirty,
        deleted,
        Some(&report.revision_id),
    )
    .expect("reconcile modified binary");

    assert_eq!(
        report.reconciled, 1,
        "the modified binary should be re-indexed"
    );
    assert_eq!(report.unchanged, 0);
    assert_eq!(report.deleted, 0);

    // It must still be stored as binary with no content — not as lossy
    // UTF-8 text.
    let row: (String, Option<String>) = connection
        .query_row(
            "SELECT file_kind, content FROM files WHERE path = 'logo.bin'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read binary row");
    assert_eq!(
        row,
        ("binary".to_owned(), None),
        "a modified binary must stay binary-excluded with no text content"
    );
}
