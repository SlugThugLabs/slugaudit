//! Tests for the metadata helpers in `manager_meta.rs`.

use super::{current_revision_id, ensure_project_row, map_publish_error, publish_from_scratch};
use crate::progress::NoopProgressSink;
use crate::store;
use crate::store::test_capture::capture_warns;
use crate::sync::publish::publish;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::path::Path;
use tempfile::TempDir;

fn open_temp_db() -> TestResult<(TempDir, rusqlite::Connection)> {
    let dir = tempfile::tempdir().check("temp dir")?;
    let db_path = dir.path().join("project.db");
    let connection = store::open_read_write(&db_path).check("open db")?;
    Ok((dir, connection))
}

#[test]
fn current_revision_id_returns_none_when_no_revision_has_been_published() -> TestResult {
    let (_dir, connection) = open_temp_db()?;
    let revision = current_revision_id(&connection).check("query")?;
    assert!(revision.is_none());
    Ok(())
}

#[test]
fn ensure_project_row_succeeds_on_a_fresh_database() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
    let root = Path::new("/tmp/example-project");
    ensure_project_row(&mut connection, root)
        .map_err(|error| format!("{error:?}"))
        .check("first ensure")?;
    // Calling twice must be idempotent (the row already exists).
    ensure_project_row(&mut connection, root)
        .map_err(|error| format!("{error:?}"))
        .check("idempotent on second call")?;
    Ok(())
}

#[test]
fn ensure_project_row_labels_a_mismatched_root_for_recovery() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
    let first_root = Path::new("/tmp/first-root");
    ensure_project_row(&mut connection, first_root)
        .map_err(|error| format!("{error:?}"))
        .check("first ensure")?;
    let second_root = Path::new("/tmp/different-root");
    let err = ensure_project_row(&mut connection, second_root).check_err("must error")?;
    // A stale database from a moved/copied repo must be reported as a
    // RootMismatch so the sync path can auto-recover by discarding and
    // rebuilding the disposable database.
    assert!(
        matches!(err, super::ProjectMetaError::RootMismatch { .. }),
        "got: {err:?}",
    );
    Ok(())
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
fn incompatible_version_is_a_stale_database_labelled_for_recovery() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
    let root = Path::new("/tmp/example-project");
    ensure_project_row(&mut connection, root)
        .map_err(|error| format!("{error:?}"))
        .check("first ensure")?;
    connection
        .execute(
            "UPDATE project SET contract_version = 99, schema_version = 99 WHERE id = 1",
            [],
        )
        .check("stamp an unsupported version")?;
    let err = ensure_project_row(&mut connection, root).check_err("must error")?;
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
    Ok(())
}

/// The publish half of the machine-readable failure contract. A spent sync
/// budget is not "often transient — retry": on a repo too big for the limit,
/// retrying unchanged fails identically, so the payload has to name the knob
/// instead. The CAS race is the opposite and
/// must stay distinguishable. A publish failure that is neither carries no
/// code at all — an over-attached code is as useless as a missing one.
#[test]
fn publish_failures_classify_into_the_documented_error_codes() -> TestResult {
    let root = Path::new("/tmp/example-project");

    let budget = map_publish_error(
        root,
        &super::publish::PublishError::TimeBudgetExceeded {
            path: String::new(),
            elapsed_ms: 4_500,
        },
        "test",
    );
    let data = budget.data.check("a spent budget carries a payload")?;
    assert_eq!(data["code"], "sync_budget_exhausted");
    let remedy = data["remedy"].as_str().check("remedy is a string")?;
    assert!(
        remedy.contains("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS"),
        "the remedy must name the knob: {remedy}"
    );
    // The same budget raised by discovery inside the publish arrives
    // wrapped, and must classify identically.
    let wrapped = map_publish_error(
        root,
        &super::publish::PublishError::Discovery(
            crate::sync::discovery::DiscoveryError::TimeBudgetExceeded { elapsed_ms: 4_500 },
        ),
        "test",
    );
    assert_eq!(
        wrapped.data.check("wrapped budget")?["code"],
        "sync_budget_exhausted"
    );

    let raced = map_publish_error(
        root,
        &super::publish::PublishError::ChangedDuringSample {
            path: "src/lib.rs".to_owned(),
        },
        "test",
    );
    assert_eq!(
        raced.data.check("the CAS race carries a payload")?["code"],
        "stale_revision"
    );

    let other = map_publish_error(
        root,
        &super::publish::PublishError::InvalidParserPackVersion("nope".to_owned()),
        "test",
    );
    assert!(
        other.data.is_none(),
        "a failure no caller can act on differently must not carry a code"
    );
    Ok(())
}

/// A failed metadata INSERT must be reported as the recording arm, not
/// misattributed to the verification read.
#[test]
fn a_failed_metadata_write_surfaces_the_recording_error() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
    connection
        .execute_batch("DROP TABLE project")
        .check("drop the project table")?;

    let error = ensure_project_row(&mut connection, Path::new("/tmp/example-project"))
        .check_err("the metadata insert must fail")?;

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
        other => {
            return Err(format!("expected ProjectMetaError::Other, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}

/// A failed metadata SELECT must be reported as the reading arm. The
/// INSERT succeeds; the authorizer denies only the `project.root_path`
/// read the verification query performs.
#[test]
fn a_failed_metadata_read_surfaces_the_reading_error() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
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
        .check_err("the metadata read must be denied")?;

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
        other => {
            return Err(format!("expected ProjectMetaError::Other, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    Ok(())
}

/// `publish_from_scratch` after corruption or staleness must surface an
/// unopenable replacement database as the recreation arm, with the store's
/// open failure preserved.
#[test]
fn a_failed_database_recreation_surfaces_the_open_error() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    let root = crate::project::ProjectRoot::resolve(project.path()).check("resolve root")?;
    let database_path = project.path().join("database-is-a-directory");
    std::fs::create_dir(&database_path).check("create the database directory")?;

    let Err(error) = publish_from_scratch(&root, database_path, &NoopProgressSink) else {
        return Err("opening a directory as a database must fail".to_owned())
            .check("unexpected outcome");
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
    Ok(())
}

/// One failed manager-path publish must emit exactly one warn line. The
/// publish log emits "publish failed" and `map_publish_error` must not log
/// the same failure again; both sites use the same message, so a duplicate
/// is counted whichever site emits it.
#[test]
fn a_failed_manager_publish_emits_exactly_one_warn_line() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").check("write source")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection =
        store::open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    connection
        .execute_batch(
            "CREATE TRIGGER once_probe_publish_failure BEFORE INSERT ON revisions \
             BEGIN SELECT RAISE(ABORT, 'injected publish failure'); END;",
        )
        .check("install the failing trigger")?;

    let (result, logs) = capture_warns(|| {
        tracing::warn!("once-probe-capture-control");
        publish(
            &mut connection,
            project.path(),
            crate::parse::PACK_VERSION,
            &NoopProgressSink,
        )
        .map_err(|error| map_publish_error(project.path(), &error, "publish failed"))
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
    Ok(())
}

/// Only `project_control on` creates `.planning/slugaudit`: a rebuild whose
/// directory vanished mid-call fails with a clear error and leaves it gone.
#[test]
fn a_rebuild_whose_activation_directory_vanished_fails_without_recreating_it() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    let activation = project.path().join(".planning").join("slugaudit");
    std::fs::create_dir_all(&activation).check("create directory")?;
    let root = crate::project::ProjectRoot::resolve(project.path()).check("resolve root")?;
    let database = crate::project::database_path(&root);
    std::fs::remove_dir_all(&activation).check("directory vanishes mid-call")?;

    let error = publish_from_scratch(&root, database, &NoopProgressSink)
        .err()
        .check("the rebuild must fail")?;

    assert!(
        error.message.contains("turned off during this call"),
        "{}",
        error.message
    );
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!("project_not_enabled")),
        "{error:?}"
    );
    assert!(!activation.exists(), "the directory is not recreated");
    Ok(())
}
