use super::*;
use crate::progress::NoopProgressSink;
use crate::store::open_read_write;
use crate::sync::publish::publish;
use crate::sync::test_support::write;
use crate::util::now_unix;

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
    let without_deleting_a =
        crate::sync::manifest::compute_manifest_hash(&connection, &[], &deletions, &[])
            .expect("hash with 40k unrelated deletions");
    deletions.push("a.rs".to_owned());

    let with_a_deleted =
        crate::sync::manifest::compute_manifest_hash(&connection, &[], &deletions, &[])
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
