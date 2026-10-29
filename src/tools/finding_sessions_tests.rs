use super::*;
use crate::tools::context::{ensure_synced_no_progress, override_session_id_for_test};
use crate::tools::test_support::activated_project;
use crate::util::SESSION_TEST_LOCK;
use rusqlite::Connection;
use uuid::Uuid;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::util::lock_or_recover(&SESSION_TEST_LOCK, "session_test_lock")
}

fn path_of(project: &tempfile::TempDir) -> String {
    project.path().to_string_lossy().into_owned()
}

fn db(project: &tempfile::TempDir) -> Connection {
    crate::store::open_read_write(
        &project
            .path()
            .join(".planning")
            .join("slugaudit")
            .join("project.db"),
    )
    .expect("open the project database")
}

/// Records one finding as `session` through the real tool path.
fn record_as(session: Uuid, project: &tempfile::TempDir, file: &str) {
    override_session_id_for_test(session);
    crate::tools::finding(
        &Parameters(crate::tools::FindingRequest {
            path: path_of(project),
            file: file.to_owned(),
            line_start: 1,
            line_end: 1,
            severity: "low".to_owned(),
            category: "correctness".to_owned(),
            title: "Reviewed".to_owned(),
            description: "checked".to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    )
    .expect("record a finding");
}

fn read_statuses(project: &tempfile::TempDir, manager: &sync::SourceSyncManager) -> Vec<String> {
    let response = crate::tools::finding_read(
        &Parameters(crate::tools::FindingReadRequest {
            path: Some(path_of(project)),
            file: None,
            cursor: None,
            legacy_paging: crate::tools::page::LegacyPaging::default(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .expect("finding_read");
    response.0.findings.into_iter().map(|f| f.status).collect()
}

fn sessions_call(
    project: &tempfile::TempDir,
    action: FindingSessionsAction,
    session: Option<String>,
    manager: &sync::SourceSyncManager,
) -> Result<FindingSessionsResponse, ErrorData> {
    finding_sessions(
        &Parameters(FindingSessionsRequest {
            path: path_of(project),
            action,
            session_id: session,
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .map(|json| json.0)
}

fn count_and_statuses(project: &tempfile::TempDir) -> (i64, Vec<String>) {
    let connection = db(project);
    let count = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count");
    let statuses = connection
        .prepare("SELECT status FROM findings ORDER BY id")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    (count, statuses)
}

#[test]
fn prior_session_findings_are_hidden_by_default_and_kept_stale() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    record_as(Uuid::new_v4(), &project, "a.rs");
    override_session_id_for_test(Uuid::new_v4());
    let fresh = sync::SourceSyncManager::default();

    assert!(read_statuses(&project, &fresh).is_empty());
    assert_eq!(count_and_statuses(&project), (1, vec!["stale".to_owned()]));
}

#[test]
fn prior_session_list_returns_id_started_at_and_count() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let earlier = Uuid::new_v4();
    record_as(earlier, &project, "a.rs");
    db(&project)
        .execute("UPDATE findings SET created_at_unix = 1234", [])
        .expect("pin the start time");
    override_session_id_for_test(Uuid::new_v4());

    let listed = sessions_call(
        &project,
        FindingSessionsAction::List,
        None,
        &sync::SourceSyncManager::default(),
    )
    .expect("list");

    assert_eq!(listed.total_sessions, 1);
    assert_eq!(listed.adopted, None);
    let [only] = listed.sessions.as_slice() else {
        panic!("expected one prior session, got {:?}", listed.sessions);
    };
    assert_eq!(only.session_id, earlier.to_string());
    assert_eq!(only.started_at_unix, 1234);
    assert_eq!(only.findings, 1);
}

#[test]
fn prior_session_adopt_makes_findings_readable_as_current() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let earlier = Uuid::new_v4();
    record_as(earlier, &project, "a.rs");
    override_session_id_for_test(Uuid::new_v4());
    let manager = sync::SourceSyncManager::default();
    assert!(read_statuses(&project, &manager).is_empty());

    let adopted = sessions_call(
        &project,
        FindingSessionsAction::Adopt,
        Some(earlier.to_string()),
        &manager,
    )
    .expect("adopt");

    assert_eq!(adopted.adopted, Some(1));
    assert_eq!(
        read_statuses(&project, &manager),
        vec!["current".to_owned()]
    );
    let listed = sessions_call(&project, FindingSessionsAction::List, None, &manager)
        .expect("list after adopt");
    assert_eq!(listed.total_sessions, 0);
}

#[test]
fn prior_session_adopt_keeps_a_changed_files_finding_stale() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let earlier = Uuid::new_v4();
    record_as(earlier, &project, "a.rs");
    std::fs::write(project.path().join("a.rs"), b"pub fn a() { 1; }\n").expect("edit");
    override_session_id_for_test(Uuid::new_v4());
    let manager = sync::SourceSyncManager::default();

    sessions_call(
        &project,
        FindingSessionsAction::Adopt,
        Some(earlier.to_string()),
        &manager,
    )
    .expect("adopt");

    assert_eq!(read_statuses(&project, &manager), vec!["stale".to_owned()]);
}

#[test]
fn prior_session_own_findings_stay_current_across_repeated_syncs() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let me = Uuid::new_v4();
    record_as(me, &project, "a.rs");
    let manager = sync::SourceSyncManager::default();

    for _ in 0..3 {
        ensure_synced_no_progress(&path_of(&project), &manager).expect("sync");
    }

    assert_eq!(
        read_statuses(&project, &manager),
        vec!["current".to_owned()]
    );
}

#[test]
fn prior_session_startup_changes_no_row_count() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    record_as(Uuid::new_v4(), &project, "a.rs");
    record_as(Uuid::new_v4(), &project, "a.rs");
    let before = count_and_statuses(&project).0;
    override_session_id_for_test(Uuid::new_v4());

    ensure_synced_no_progress(&path_of(&project), &sync::SourceSyncManager::default())
        .expect("startup sync");

    let (after, statuses) = count_and_statuses(&project);
    assert_eq!(after, before);
    assert_eq!(statuses, vec!["stale".to_owned(), "stale".to_owned()]);
}

#[test]
fn prior_session_adopt_requires_a_session_id() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let error = sessions_call(
        &project,
        FindingSessionsAction::Adopt,
        None,
        &sync::SourceSyncManager::default(),
    )
    .expect_err("missing session_id");
    assert_eq!(
        error.message,
        "action `adopt` requires session_id; call `list` first"
    );
}

#[test]
fn prior_session_adopt_refuses_the_current_session() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let me = Uuid::new_v4();
    override_session_id_for_test(me);
    let error = sessions_call(
        &project,
        FindingSessionsAction::Adopt,
        Some(me.to_string()),
        &sync::SourceSyncManager::default(),
    )
    .expect_err("own session");
    assert_eq!(
        error.message,
        "session_id is the current session; its findings are already yours"
    );
}

#[test]
fn prior_session_list_refuses_a_session_id() {
    let _guard = lock();
    let project = activated_project("a.rs", b"pub fn a() {}\n");
    let error = sessions_call(
        &project,
        FindingSessionsAction::List,
        Some("x".to_owned()),
        &sync::SourceSyncManager::default(),
    )
    .expect_err("list with id");
    assert_eq!(
        error.message,
        "session_id is only valid with action `adopt`"
    );
}
