use super::*;
use std::fs;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

#[test]
fn a_verified_connection_succeeds_when_nothing_changed_since_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let synced = ensure_synced(&project.path().to_string_lossy()).expect("sync");

    let connection = open_verified_read_only(&synced).expect("still-fresh revision opens fine");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .expect("query");
    assert_eq!(count, 1);
}

#[test]
fn a_stale_synced_handle_fails_loudly_instead_of_returning_mismatched_data() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let stale = ensure_synced(&path).expect("first sync");

    // Simulate a concurrent publish from another process: modify the file
    // and sync again independently of `stale`.
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let fresh = ensure_synced(&path).expect("second sync");
    assert_ne!(
        stale.revision_id, fresh.revision_id,
        "the revision must actually have moved"
    );

    let result = open_verified_read_only(&stale);
    assert!(
        result.is_err(),
        "a stale revision handle must never silently open against newer data"
    );
}

#[test]
fn verified_read_write_has_the_same_protection() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let stale = ensure_synced(&path).expect("first sync");

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    ensure_synced(&path).expect("second sync");

    assert!(open_verified_read_write(&stale).is_err());
}
