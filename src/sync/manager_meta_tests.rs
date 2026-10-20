//! Tests for the metadata helpers split out of `manager.rs`.

use super::{
    current_revision_id, ensure_project_row, map_publish_error, publish_from_scratch, publish_full,
    purge_prior_session_findings,
};
use crate::progress::NoopProgressSink;
use crate::store;
use crate::store::test_capture::capture_warns;
use std::path::Path;
use tempfile::TempDir;
use uuid::Uuid;

fn open_temp_db() -> (TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("project.db");
    let connection = store::open_read_write(&db_path).expect("open db");
    (dir, connection)
}

#[test]
fn current_revision_id_returns_none_when_no_revision_has_been_published() {
    let (_dir, connection) = open_temp_db();
    let revision = current_revision_id(&connection).expect("query");
    assert!(revision.is_none());
}

#[test]
fn ensure_project_row_succeeds_on_a_fresh_database() {
    let (_dir, mut connection) = open_temp_db();
    let root = Path::new("/tmp/example-project");
    ensure_project_row(&mut connection, root).expect("first ensure");
    // Calling twice must be idempotent (the row already exists).
    ensure_project_row(&mut connection, root).expect("idempotent on second call");
}

#[test]
fn ensure_project_row_labels_a_mismatched_root_for_recovery() {
    let (_dir, mut connection) = open_temp_db();
    let first_root = Path::new("/tmp/first-root");
    ensure_project_row(&mut connection, first_root).expect("first ensure");
    let second_root = Path::new("/tmp/different-root");
    let err = ensure_project_row(&mut connection, second_root).expect_err("must error");
    // A stale database from a moved/copied repo must be reported as a
    // RootMismatch so the sync path can auto-recover by discarding and
    // rebuilding the disposable database.
    assert!(
        matches!(err, super::ProjectMetaError::RootMismatch { .. }),
        "got: {err:?}",
    );
}

#[test]
fn root_mismatch_converts_to_an_actionable_message() {
    let err: rmcp::ErrorData = super::ProjectMetaError::RootMismatch {
        stored_root: "/old/path".into(),
    }
    .into();
    assert!(
        err.message.contains("built for a different project root")
            && err.message.contains("/old/path"),
        "got: {err:?}",
    );
}

#[test]
fn incompatible_version_is_a_stale_database_labelled_for_recovery() {
    let (_dir, mut connection) = open_temp_db();
    let root = Path::new("/tmp/example-project");
    ensure_project_row(&mut connection, root).expect("first ensure");
    connection
        .execute(
            "UPDATE project SET contract_version = 99, schema_version = 99 WHERE id = 1",
            [],
        )
        .expect("stamp an unsupported version");
    let err = ensure_project_row(&mut connection, root).expect_err("must error");
    assert!(
        matches!(err, super::ProjectMetaError::IncompatibleVersion { .. }),
        "got: {err:?}",
    );

    let err: rmcp::ErrorData = super::ProjectMetaError::IncompatibleVersion {
        contract_version: 99,
        schema_version: 99,
    }
    .into();
    assert!(
        err.message.contains("unsupported SlugAudit") && err.message.contains("99/99"),
        "got: {err:?}",
    );
}

/// Drives `purge_prior_session_findings` directly with two distinct
/// session UUIDs against a hand-seeded findings table, simulating two
/// separate agent processes. The first session's rows must be deleted;
/// the second session's rows must remain untouched. This is the
/// cross-session poisoning defense's correctness surface — no other
/// test exercises the live SESSION_ID flow without going through
/// `finding()`, which would couple the test to the schema panel.
#[test]
fn purge_deletes_only_prior_session_rows() {
    let (_dir, mut connection) = open_temp_db();
    for (path, session) in [
        ("old.rs", "11111111-1111-1111-1111-111111111111"),
        ("shared.rs", "22222222-2222-2222-2222-222222222222"),
    ] {
        connection
            .execute(
                "INSERT INTO findings (path, source_hash, line_start, line_end, \
                 severity, category, title, description, created_at_unix, \
                 evidence_revision, status, session_id) \
                 VALUES (?1, 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', ?2)",
                rusqlite::params![path, session],
            )
            .expect("seed finding");
    }

    // A second agent boots with its own UUID and runs the purge.
    purge_prior_session_findings(
        &mut connection,
        Uuid::parse_str("33333333-3333-3333-3333-333333333333").expect("new uuid"),
    )
    .expect("purge");

    let remaining: Vec<String> = connection
        .prepare("SELECT session_id FROM findings ORDER BY path")
        .expect("select")
        .query_map([], |row| row.get(0))
        .expect("query")
        .map(|value| value.expect("row"))
        .collect();
    // The "1111" row was tagged with a session other than the new
    // "3333" session, so the cleanup MUST drop it. The "2222" row was
    // also tagged as "not me", so it MUST drop too — only rows stamped
    // with the CURRENT session survive.
    assert!(
        remaining.is_empty(),
        "every legacy row is from a prior session and must be gone: {remaining:?}"
    );

    // A second run with a session that DOES own a row is a no-op
    // (no rows deleted) — the test pins that the cleanup is exact,
    // not over-broad.
    connection
        .execute(
            "INSERT INTO findings (path, source_hash, line_start, line_end, \
             severity, category, title, description, created_at_unix, \
             evidence_revision, status, session_id) \
             VALUES ('alive.rs', 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', ?1)",
            rusqlite::params![
                Uuid::parse_str("44444444-4444-4444-4444-444444444444")
                    .expect("alive uuid")
                    .to_string()
            ],
        )
        .expect("insert alive");
    purge_prior_session_findings(
        &mut connection,
        Uuid::parse_str("44444444-4444-4444-4444-444444444444").expect("alive uuid"),
    )
    .expect("no-op purge");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "alive row must survive a matching-session purge");
}

/// `ensure_project_row` is metadata-only: the findings purge belongs to
/// the session-start transition owned by `SourceSyncManager`, not to the
/// per-sync metadata row. If this function purged, every tool call would
/// delete a concurrent live session's findings.
#[test]
fn ensure_project_row_leaves_findings_to_the_session_start_transition() {
    let _session_guard = crate::util::lock_or_recover(&SESSION_TEST_LOCK, "session_test_lock");
    let (_dir, mut connection) = open_temp_db();

    let root = Path::new("/tmp/meta-only-example");
    ensure_project_row(&mut connection, root).expect("first ensure");

    let prior_session =
        Uuid::parse_str("55555555-5555-5555-5555-555555555555").expect("prior uuid");
    connection
        .execute(
            "INSERT INTO findings (path, source_hash, line_start, line_end, \
             severity, category, title, description, created_at_unix, \
             evidence_revision, status, session_id) \
             VALUES ('stale.rs', 'h', 1, 1, 'low', 'c', 't', 'd', 0, 'r', 'current', ?1)",
            rusqlite::params![prior_session.to_string()],
        )
        .expect("seed prior-session finding");

    ensure_project_row(&mut connection, root).expect("second ensure");

    let count: i64 = connection
        .query_row("SELECT count(*) FROM findings", [], |row| row.get(0))
        .expect("count");
    assert_eq!(
        count, 1,
        "the metadata row helper must not delete another session's findings"
    );
}

/// The publish half of the machine-readable failure contract. A spent sync
/// budget is the case that used to read as "often transient — retry": on a
/// repo too big for the limit, retrying unchanged fails identically, so the
/// payload has to name the knob instead. The CAS race is the opposite and
/// must stay distinguishable. A publish failure that is neither carries no
/// code at all — an over-attached code is as useless as a missing one.
#[test]
fn publish_failures_classify_into_the_documented_error_codes() {
    let root = Path::new("/tmp/example-project");

    let budget = map_publish_error(
        root,
        super::publish::PublishError::TimeBudgetExceeded {
            path: String::new(),
            elapsed_ms: 4_500,
        },
        "test",
    );
    let data = budget.data.expect("a spent budget carries a payload");
    assert_eq!(data["code"], "sync_budget_exhausted");
    let remedy = data["remedy"].as_str().expect("remedy is a string");
    assert!(
        remedy.contains("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS"),
        "the remedy must name the knob: {remedy}"
    );
    // The same budget raised by discovery inside the publish arrives
    // wrapped, and must classify identically.
    let wrapped = map_publish_error(
        root,
        super::publish::PublishError::Discovery(
            crate::sync::discovery::DiscoveryError::TimeBudgetExceeded { elapsed_ms: 4_500 },
        ),
        "test",
    );
    assert_eq!(
        wrapped.data.expect("wrapped budget")["code"],
        "sync_budget_exhausted"
    );

    let raced = map_publish_error(
        root,
        super::publish::PublishError::ChangedDuringSample {
            path: "src/lib.rs".to_owned(),
        },
        "test",
    );
    assert_eq!(
        raced.data.expect("the CAS race carries a payload")["code"],
        "stale_revision"
    );

    let other = map_publish_error(
        root,
        super::publish::PublishError::InvalidParserPackVersion("nope".to_owned()),
        "test",
    );
    assert!(
        other.data.is_none(),
        "a failure no caller can act on differently must not carry a code"
    );
}

/// A damaged `findings` table must surface the purge failure as its own
/// `ErrorData`, with the SQLite cause preserved, rather than being folded
/// into one of the later metadata arms.
#[test]
fn a_failed_prior_session_purge_surfaces_the_purge_error() {
    let (_dir, mut connection) = open_temp_db();
    connection
        .execute_batch("DROP TABLE findings")
        .expect("drop the findings table");

    let error = purge_prior_session_findings(
        &mut connection,
        Uuid::parse_str("77777777-7777-7777-7777-777777777777").expect("uuid"),
    )
    .expect_err("the purge must fail on a damaged schema");

    assert!(
        error
            .message
            .starts_with("purging prior-session findings: "),
        "got: {}",
        error.message
    );
    assert!(
        error.message.contains("no such table: findings"),
        "the source error must be preserved: {}",
        error.message
    );
}

/// A failed metadata INSERT must be reported as the recording arm, not
/// misattributed to the purge or the verification read.
#[test]
fn a_failed_metadata_write_surfaces_the_recording_error() {
    let (_dir, mut connection) = open_temp_db();
    connection
        .execute_batch("DROP TABLE project")
        .expect("drop the project table");

    let error = ensure_project_row(&mut connection, Path::new("/tmp/example-project"))
        .expect_err("the metadata insert must fail");

    match error {
        super::ProjectMetaError::Other(error) => {
            assert!(
                error.message.starts_with("recording project metadata: "),
                "got: {}",
                error.message
            );
            assert!(
                error.message.contains("no such table: project"),
                "the source error must be preserved: {}",
                error.message
            );
        }
        other => panic!("expected ProjectMetaError::Other, got {other:?}"),
    }
}

/// A failed metadata SELECT must be reported as the reading arm. The
/// INSERT succeeds; the authorizer denies only the `project.root_path`
/// read the verification query performs.
#[test]
fn a_failed_metadata_read_surfaces_the_reading_error() {
    let (_dir, mut connection) = open_temp_db();
    connection.authorizer(Some(
        |context: rusqlite::hooks::AuthContext<'_>| match context.action {
            rusqlite::hooks::AuthAction::Read {
                table_name,
                column_name,
            } if table_name == "project" && column_name == "root_path" => {
                rusqlite::hooks::Authorization::Deny
            }
            _ => rusqlite::hooks::Authorization::Allow,
        },
    ));

    let error = ensure_project_row(&mut connection, Path::new("/tmp/example-project"))
        .expect_err("the metadata read must be denied");

    match error {
        super::ProjectMetaError::Other(error) => {
            assert!(
                error.message.starts_with("reading project metadata: "),
                "got: {}",
                error.message
            );
            assert!(
                error
                    .message
                    .contains("access to project.root_path is prohibited"),
                "the source error must be preserved: {}",
                error.message
            );
        }
        other => panic!("expected ProjectMetaError::Other, got {other:?}"),
    }
}

/// `publish_from_scratch` after corruption or staleness must surface an
/// unopenable replacement database as the recreation arm, with the store's
/// open failure preserved.
#[test]
fn a_failed_database_recreation_surfaces_the_open_error() {
    use std::os::unix::fs::PermissionsExt;

    let project = tempfile::tempdir().expect("project dir");
    let root = crate::project::ProjectRoot::resolve(project.path()).expect("resolve root");
    let database_path = project.path().join("database-is-a-directory");
    std::fs::create_dir(&database_path).expect("create the database directory");
    // Owner-only, so the store's permission guard passes and the open
    // itself is what fails.
    std::fs::set_permissions(&database_path, std::fs::Permissions::from_mode(0o700))
        .expect("restrict the directory mode");

    let error = match publish_from_scratch(&root, database_path, &NoopProgressSink) {
        Ok(_) => panic!("opening a directory as a database must fail"),
        Err(error) => error,
    };

    assert!(
        error
            .message
            .starts_with("recreating the project database: "),
        "got: {}",
        error.message
    );
    assert!(
        error.message.contains("failed to open database"),
        "the store's open failure must be preserved: {}",
        error.message
    );
}

/// One failed manager-path publish must emit exactly one warn line. The
/// publish log emits "publish failed" and `map_publish_error` used to log
/// the same failure again; both sites use the same message, so a duplicate
/// is counted whichever site emits it.
#[test]
fn a_failed_manager_publish_emits_exactly_one_warn_line() {
    let project = tempfile::tempdir().expect("project dir");
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write source");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection =
        store::open_read_write(&db_dir.path().join("project.db")).expect("open db");
    connection
        .execute_batch(
            "CREATE TRIGGER once_probe_publish_failure BEFORE INSERT ON revisions \
             BEGIN SELECT RAISE(ABORT, 'injected publish failure'); END;",
        )
        .expect("install the failing trigger");

    let (result, logs) = capture_warns(|| {
        tracing::warn!("once-probe-capture-control");
        publish_full(
            &mut connection,
            project.path(),
            &NoopProgressSink,
            "publish failed",
        )
    });

    assert!(
        logs.contains("once-probe-capture-control"),
        "the capture must be live: {logs}"
    );
    let warnings = logs
        .lines()
        .filter(|line| line.contains("publish failed"))
        .count();
    assert_eq!(warnings, 1, "one failed publish, one warn line: {logs}");
    assert!(result.is_err(), "the trigger must fail the publish");
}
