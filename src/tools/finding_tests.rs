use super::*;
use crate::sync;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use std::fs;

fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

fn base_request(project: &tempfile::TempDir, file: &str) -> FindingRequest {
    FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: file.to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "medium".to_owned(),
        category: "correctness".to_owned(),
        title: "Reviewed conclusion".to_owned(),
        description: "A conclusion the AI actually checked.".to_owned(),
    }
}

fn finding_status(connection: &Connection, id: i64) -> String {
    connection
        .query_row("SELECT status FROM findings WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("read finding status")
}

#[test]
fn persists_exactly_the_supplied_conclusion() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let response =
        finding(&Parameters(base_request(&project, "lib.rs"))).expect("finding succeeds");

    assert_eq!(response.0.status, "current");
    assert!(!response.0.source_hash.is_empty());
    assert!(response.0.id > 0);
}

#[test]
fn a_modified_file_invalidates_its_finding_on_next_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let response =
        finding(&Parameters(base_request(&project, "lib.rs"))).expect("finding succeeds");
    let id = response.0.id;

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .expect("modify file");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(&mut connection, project.path(), "1.0.0").expect("resync");

    assert_eq!(finding_status(&connection, id), "stale");
}

#[test]
fn a_deleted_file_invalidates_its_finding_on_next_sync() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let response =
        finding(&Parameters(base_request(&project, "lib.rs"))).expect("finding succeeds");
    let id = response.0.id;

    fs::remove_file(project.path().join("lib.rs")).expect("delete file");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(&mut connection, project.path(), "1.0.0").expect("resync");

    assert_eq!(finding_status(&connection, id), "stale");
}

#[test]
fn an_untouched_file_keeps_its_finding_current() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    fs::write(project.path().join("other.rs"), b"pub fn b() {}\n").expect("write second file");
    let response =
        finding(&Parameters(base_request(&project, "lib.rs"))).expect("finding succeeds");
    let id = response.0.id;

    fs::write(
        project.path().join("other.rs"),
        b"pub fn b() { changed(); }\n",
    )
    .expect("modify other file");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(&mut connection, project.path(), "1.0.0").expect("resync");

    assert_eq!(finding_status(&connection, id), "current");
}

#[test]
fn sync_never_creates_a_finding_on_its_own() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let mut connection = crate::store::open_read_write(&db_path).expect("open db");
    sync::publish(&mut connection, project.path(), "1.0.0").expect("sync");

    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count findings");
    assert_eq!(count, 0);
}

#[test]
fn empty_title_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = base_request(&project, "lib.rs");
    request.title = String::new();
    assert!(finding(&Parameters(request)).is_err());
}

#[test]
fn reversed_line_range_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 10;
    request.line_end = 1;
    assert!(finding(&Parameters(request)).is_err());
}

#[test]
fn a_finding_against_an_unindexed_file_is_a_typed_error() {
    let project = activated_project("lib.rs", b"pub fn a() {}\n");
    let request = base_request(&project, "does_not_exist.rs");
    assert!(finding(&Parameters(request)).is_err());
}
