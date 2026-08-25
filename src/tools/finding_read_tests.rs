use super::*;
use crate::store;
use crate::sync;
use crate::tools::context::ensure_synced_no_progress;
use crate::tools::test_support::activated_project;
use crate::util::SESSION_TEST_LOCK;
use rmcp::handler::server::wrapper::Parameters;

/// Acquires the shared [`SESSION_TEST_LOCK`] and runs `f`. Every test
/// here both writes findings (stamped with the process-global
/// `session_id()`) and reads them back scoped by that same id; it also
/// calls `finding_read`, whose `ensure_synced` runs the session-scoped
/// purge (`DELETE FROM findings WHERE session_id != ?`). Without the
/// lock, a sibling session-flipping test (one calling
/// `override_session_id_for_test`) could flip the global between this
/// test's write and read — making the read come back empty — or
/// trigger the purge against this test's rows. Holding the shared lock
/// for the whole write→sync→read cycle keeps the session stable and
/// keeps the purge from ever targeting these rows.
fn with_stable_session<T>(f: impl FnOnce() -> T) -> T {
    let _guard = SESSION_TEST_LOCK.lock().expect("session test lock poisoned");
    f()
}

fn request(path: &str, file: Option<&str>) -> FindingReadRequest {
    FindingReadRequest {
        path: path.to_owned(),
        file: file.map(str::to_owned),
    }
}

fn db_path(project: &tempfile::TempDir) -> std::path::PathBuf {
    project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db")
}

fn write_finding(
    project: &tempfile::TempDir,
    file: &str,
    title: &str,
    manager: &sync::SourceSyncManager,
) -> crate::tools::FindingResponse {
    crate::tools::finding(
        &Parameters(crate::tools::FindingRequest {
            path: project.path().to_string_lossy().into_owned(),
            file: file.to_owned(),
            line_start: 1,
            line_end: 1,
            severity: "low".to_owned(),
            category: "test".to_owned(),
            title: title.to_owned(),
            description: "d".to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .expect("write finding")
    .0
}

#[test]
fn an_empty_session_returns_no_findings() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();

        let response = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("finding_read");
        assert!(response.0.findings.is_empty());
    });
}

#[test]
fn finding_persists_and_is_readable() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();

        let finding = write_finding(&project, "lib.rs", "test finding", &manager);
        assert!(finding.id > 0);

        // Open read-only to verify the row is in the DB before going
        // through finding_read (which calls ensure_synced and purge).
        let conn = store::open_read_only(&db_path(&project)).expect("open db");
        let count: i64 = conn
            .query_row("SELECT count(*) FROM findings", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1, "finding must be in the DB");

        let response = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("finding_read");
        assert_eq!(response.0.findings.len(), 1);
        assert_eq!(response.0.findings[0].title, "test finding");
    });
}

#[test]
fn file_filter_returns_only_that_file() {
    with_stable_session(|| {
        let project = activated_project("a.rs", b"fn a() {}\n");
        std::fs::write(project.path().join("b.rs"), b"fn b() {}\n").expect("b.rs");
        let manager = sync::SourceSyncManager::default();

        // Index b.rs so the finding tool can reference it.
        ensure_synced_no_progress(&project.path().to_string_lossy(), &manager).expect("initial sync");

        write_finding(&project, "a.rs", "a finding", &manager);
        write_finding(&project, "b.rs", "b finding", &manager);

        let conn = store::open_read_only(&db_path(&project)).expect("open db");
        let count: i64 = conn
            .query_row("SELECT count(*) FROM findings", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 2, "both findings present");

        let filtered = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), Some("a.rs"))),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("filtered");
        assert_eq!(filtered.0.findings.len(), 1);
        assert_eq!(filtered.0.findings[0].path, "a.rs");
    });
}

#[test]
fn stale_findings_are_returned_with_stale_status() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();

        write_finding(&project, "lib.rs", "about to go stale", &manager);

        let before = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("before");
        assert_eq!(before.0.findings[0].status, "current");

        std::fs::write(project.path().join("lib.rs"), b"fn main() { changed(); }\n").expect("modify");
        ensure_synced_no_progress(&project.path().to_string_lossy(), &manager).expect("re-sync");

        let after = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("after");
        assert_eq!(after.0.findings[0].status, "stale");
    });
}
