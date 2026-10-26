use super::*;
use crate::store;
use crate::sync;
use crate::tools::context::ensure_synced_no_progress;
use crate::tools::test_support::activated_project;
use crate::util::SESSION_TEST_LOCK;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;

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
    let _guard = crate::util::lock_or_recover(&SESSION_TEST_LOCK, "session_test_lock");
    f()
}

fn request(path: &str, file: Option<&str>) -> FindingReadRequest {
    FindingReadRequest {
        path: path.to_owned(),
        file: file.map(str::to_owned),
        offset: 0,
        expected_revision: None,
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
    write_finding_with_description(project, file, title, "d", manager)
}

fn write_finding_with_description(
    project: &tempfile::TempDir,
    file: &str,
    title: &str,
    description: &str,
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
            description: description.to_owned(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .expect("write finding")
    .0
}

/// Writes the project-config key `resolve_limits` reads, so a test can set
/// a page budget without an env var (`process_limits` is a `OnceLock`).
fn write_default_page_bytes(project: &tempfile::TempDir, page_bytes: usize) {
    let config = db_path(project)
        .parent()
        .expect("database has a parent")
        .join("config.json");
    std::fs::write(config, format!("{{\"default_page_bytes\": {page_bytes}}}"))
        .expect("write project config");
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
        ensure_synced_no_progress(&project.path().to_string_lossy(), &manager)
            .expect("initial sync");

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

        std::fs::write(project.path().join("lib.rs"), b"fn main() { changed(); }\n")
            .expect("modify");
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

#[test]
fn an_oversized_read_path_or_file_is_a_typed_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let base = directory.path().to_string_lossy().into_owned();

    let oversized_path = FindingReadRequest {
        path: "x".repeat(MAX_PATH_BYTES + 1),
        file: None,
        offset: 0,
        expected_revision: None,
    };
    let Err(path_error) = finding_read(
        &Parameters(oversized_path),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    ) else {
        panic!("an oversized path must be refused before resolution");
    };
    assert_eq!(
        path_error.message,
        "path or file exceeds its input size limit"
    );
    assert_eq!(path_error.code, rmcp::model::ErrorCode::INVALID_PARAMS);

    let oversized_file = FindingReadRequest {
        path: base,
        file: Some("x".repeat(MAX_FILE_BYTES + 1)),
        offset: 0,
        expected_revision: None,
    };
    let Err(file_error) = finding_read(
        &Parameters(oversized_file),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    ) else {
        panic!("an oversized file filter must be refused");
    };
    assert_eq!(
        file_error.message,
        "path or file exceeds its input size limit"
    );
    assert_eq!(file_error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[test]
fn finding_read_refuses_an_offset_past_the_supported_maximum() {
    // SQLite reads an integer literal too large for i64 as a REAL, which
    // would page from an unpredictable position instead of failing — the
    // current session's rows would come back wrong with no error.
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();
        write_finding(&project, "lib.rs", "paging guard", &manager);

        let at = |offset: usize| {
            finding_read(
                &Parameters(FindingReadRequest {
                    path: project.path().to_string_lossy().into_owned(),
                    file: None,
                    offset,
                    expected_revision: None,
                }),
                &crate::progress::NoopProgressSink,
                &manager,
            )
            .map(|Json(response)| response)
        };

        let error = at(i64::MAX as usize + 1).expect_err("an out-of-range offset is refused");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(
            error.message,
            format!(
                "offset {} exceeds the maximum supported page offset ({}); \
                 page from the start or from a returned next_offset",
                i64::MAX as usize + 1,
                i64::MAX
            ),
            "the guard names the offending value and the bound",
        );

        // The bound itself is still usable and yields no rows at all, not
        // rows from a floating-point position.
        let page = at(i64::MAX as usize).expect("the maximum offset is accepted");
        assert!(page.findings.is_empty(), "no finding lives at that offset");
        assert!(page.page.next_offset.is_none());
    });
}

#[test]
fn a_missing_findings_table_is_an_internal_error() {
    let mut connection = Connection::open_in_memory().expect("in-memory database");
    let tx = connection.transaction().expect("transaction");

    let error = read_session_findings(&tx, ("session", None), 0, 1_000)
        .expect_err("the statement cannot prepare against a missing table");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.contains("no such table: findings"),
        "{}",
        error.message
    );
}

#[test]
fn an_undecodable_finding_row_is_an_internal_error() {
    let mut connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(
            "CREATE TABLE findings (id INTEGER, path INTEGER, line_start INTEGER, line_end INTEGER, \
             severity TEXT, category TEXT, title TEXT, description TEXT, status TEXT, \
             source_hash TEXT, session_id TEXT); \
             INSERT INTO findings VALUES (1, 42, 1, 1, 'low', 'test', 't', 'd', 'current', \
             'hash', 'session');",
        )
        .expect("create test schema");
    let tx = connection.transaction().expect("transaction");

    let error = read_session_findings(&tx, ("session", None), 0, 10_000)
        .expect_err("a TEXT column holding an integer cannot decode into a String");

    assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert!(
        error.message.contains("Invalid column type") && error.message.contains("path"),
        "{}",
        error.message
    );
}

/// A page is cut by serialized bytes, resumed by `next_offset`, and the
/// pages together are exactly the stored rows: no row is lost to the budget
/// and none is returned twice.
#[test]
fn finding_read_pages_by_byte_budget_without_losing_or_duplicating_rows() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();
        let description = "d".repeat(10_000);
        let mut written = Vec::new();
        for index in 0..5 {
            let finding = write_finding_with_description(
                &project,
                "lib.rs",
                &format!("finding {index}"),
                &description,
                &manager,
            );
            written.push(finding.id);
        }
        write_default_page_bytes(&project, 15_000);

        let mut offset = 0usize;
        let mut seen = Vec::new();
        let mut pages = 0usize;
        loop {
            let response = finding_read(
                &Parameters(FindingReadRequest {
                    path: project.path().to_string_lossy().into_owned(),
                    file: None,
                    offset,
                    expected_revision: None,
                }),
                &crate::progress::NoopProgressSink,
                &manager,
            )
            .expect("page")
            .0;
            let page = &response.page;
            assert_eq!(page.page_bytes, 15_000, "the configured page applies");
            assert!(
                page.bytes_used <= page.page_bytes,
                "page {} spent {} bytes of a {} budget",
                pages,
                page.bytes_used,
                page.page_bytes
            );
            assert_eq!(page.returned, 1, "returned counts this page's findings");
            assert_eq!(
                page.total,
                Some(5),
                "total counts every finding the read selects"
            );
            assert_eq!(page.complete, page.next_offset.is_none());
            let expected_cut = if page.complete {
                crate::tools::PageLimit::None
            } else {
                crate::tools::PageLimit::PageBytes
            };
            assert_eq!(
                page.limited_by, expected_cut,
                "a cut page names the byte budget"
            );
            assert_eq!(response.findings.len(), 1, "one 10 KB row per 15 KB page");
            assert!(
                !response.findings[0].detail_clipped,
                "a row that fits must not be clipped"
            );
            seen.extend(response.findings.iter().map(|entry| entry.id));
            pages += 1;
            match response.page.next_offset {
                Some(next) => {
                    assert!(next > offset, "a page must always make progress");
                    offset = next;
                }
                None => break,
            }
        }

        assert_eq!(pages, 5, "every row needs its own page");
        assert_eq!(
            seen, written,
            "the pages must be the stored rows in id order, exactly once each"
        );
    });
}

/// A description too large for the budget is clipped on a character
/// boundary, flagged, and the clipped page still fits the budget.
#[test]
fn an_oversized_description_is_clipped_on_a_char_boundary_and_flagged() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();
        // Five thousand characters, ten thousand UTF-8 bytes: a clip must
        // never split one of these two-byte characters.
        let description = "é".repeat(5_000);
        write_finding_with_description(&project, "lib.rs", "multibyte", &description, &manager);
        write_default_page_bytes(&project, 500);

        let response = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("read")
        .0;

        assert_eq!(response.findings.len(), 1);
        let entry = &response.findings[0];
        assert!(entry.detail_clipped, "the clip must be reported");
        assert!(
            entry.description.chars().count() < description.chars().count(),
            "the stored description must actually have been shortened"
        );
        assert!(
            description.starts_with(&entry.description),
            "the returned text must be a character-boundary prefix"
        );
        assert!(
            response.page.bytes_used <= response.page.page_bytes,
            "a clipped page must fit its budget: {} of {}",
            response.page.bytes_used,
            response.page.page_bytes
        );
        assert!(
            response.page.next_offset.is_none(),
            "the scope was exhausted"
        );
        assert!(response.page.complete, "the scope was exhausted");
    });
}

/// A resumed page pinned to a revision the index has moved past is refused
/// with `stale_revision`, as on every other paging tool, instead of paging an
/// offset taken against another revision.
#[test]
fn finding_read_refuses_a_resumed_page_whose_expected_revision_is_stale() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();
        write_finding(&project, "lib.rs", "pinned", &manager);
        let first = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("first page")
        .0;
        std::fs::write(project.path().join("lib.rs"), b"fn main() { moved(); }\n").expect("modify");

        let error = finding_read(
            &Parameters(FindingReadRequest {
                expected_revision: Some(first.page.revision_id.clone()),
                ..request(&project.path().to_string_lossy(), None)
            }),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .map(|Json(response)| response)
        .expect_err("a stale pinned revision is refused");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let code = error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(code, Some("stale_revision"), "{error:?}");
    });
}

/// A pin that still names the current revision is honoured: the page comes
/// back under that same revision.
#[test]
fn finding_read_serves_a_resumed_page_whose_expected_revision_is_current() {
    with_stable_session(|| {
        let project = activated_project("lib.rs", b"fn main() {}\n");
        let manager = sync::SourceSyncManager::default();
        write_finding(&project, "lib.rs", "pinned", &manager);
        let first = finding_read(
            &Parameters(request(&project.path().to_string_lossy(), None)),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("first page")
        .0;

        let resumed = finding_read(
            &Parameters(FindingReadRequest {
                expected_revision: Some(first.page.revision_id.clone()),
                ..request(&project.path().to_string_lossy(), None)
            }),
            &crate::progress::NoopProgressSink,
            &manager,
        )
        .expect("a current pin is served")
        .0;

        assert_eq!(resumed.page.revision_id, first.page.revision_id);
        assert_eq!(resumed.page.total, Some(1));
    });
}

#[test]
fn an_oversized_finding_read_expected_revision_is_refused() {
    let directory = tempfile::tempdir().expect("temp dir");
    let oversized = FindingReadRequest {
        expected_revision: Some("r".repeat(MAX_REVISION_BYTES + 1)),
        ..request(&directory.path().to_string_lossy(), None)
    };

    let error = finding_read(
        &Parameters(oversized),
        &crate::progress::NoopProgressSink,
        &sync::SourceSyncManager::default(),
    )
    .map(|Json(response)| response)
    .expect_err("an oversized expected_revision is refused before resolution");

    assert_eq!(error.message, "expected_revision exceeds 256 bytes");
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}
