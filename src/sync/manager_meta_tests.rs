//! Tests for project metadata SQL helpers (`project_meta.rs`) and manager
//! publish-error / rebuild helpers (`manager.rs`).

use super::ensure_project_row;
use crate::progress::NoopProgressSink;
use crate::store;
use crate::store::test_capture::capture_warns;
use crate::sync::manager::{map_publish_error, publish_from_scratch};
use crate::sync::publish::{self, publish};
use crate::sync::revision::current_revision_id;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
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
    ensure(
        revision.is_none(),
        "an empty database has no current revision",
    )?;
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
    ensure(
        matches!(err, super::ProjectMetaError::RootMismatch { .. }),
        format!("got: {err:?}"),
    )?;
    Ok(())
}

#[test]
fn root_mismatch_converts_to_an_actionable_message() -> TestResult {
    let err: rmcp::ErrorData = super::ProjectMetaError::RootMismatch {
        stored_root: "/old/path".into(),
    }
    .into();
    ensure(
        err.message.contains("built for a different project root")
            && err.message.contains("/old/path"),
        format!("got: {err:?}"),
    )?;
    Ok(())
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
    ensure(
        matches!(err, super::ProjectMetaError::IncompatibleVersion { .. }),
        format!("got: {err:?}"),
    )?;

    let err: rmcp::ErrorData = super::ProjectMetaError::IncompatibleVersion {
        contract_version: 99,
        schema_version: 99,
    }
    .into();
    ensure(
        err.message.contains("unsupported SlugAudit") && err.message.contains("99/99"),
        format!("got: {err:?}"),
    )?;
    Ok(())
}

/// The publish half of the machine-readable failure contract. A spent sync
/// budget is not "often transient — retry": on a repo too big for the limit,
/// retrying unchanged fails identically, so the payload has to tell the model
/// how to switch to the audit profile instead (without leaking env/config).
/// The CAS race is the opposite and must stay distinguishable. A publish
/// failure that is neither carries no code at all — an over-attached code is as
/// useless as a missing one.
#[test]
fn publish_failures_classify_into_the_documented_error_codes() -> TestResult {
    let root = Path::new("/tmp/example-project");

    let budget = map_publish_error(
        root,
        &publish::PublishError::TimeBudgetExceeded {
            activity: "diffing",
            path: None,
            elapsed_ms: 4_500,
        },
        "test",
    );
    let data = budget.data.check("a spent budget carries a payload")?;
    ensure_eq(
        data.get("code").check("the budget payload has a code")?,
        &"sync_budget_exhausted",
        "spent sync budget carries the sync_budget_exhausted code",
    )?;
    let remedy = data
        .get("remedy")
        .and_then(serde_json::Value::as_str)
        .check("remedy is a string")?;
    ensure(
        remedy.contains("audit profile")
            && remedy.contains("project_control")
            && !remedy.contains("SLUGAUDIT_"),
        format!("the remedy must name the audit profile switch and never SLUGAUDIT_*: {remedy}"),
    )?;
    // The same budget raised by discovery inside the publish arrives
    // wrapped, and must classify identically.
    let wrapped = map_publish_error(
        root,
        &publish::PublishError::Discovery(
            crate::sync::discovery::DiscoveryError::TimeBudgetExceeded { elapsed_ms: 4_500 },
        ),
        "test",
    );
    ensure_eq(
        wrapped
            .data
            .check("wrapped budget")?
            .get("code")
            .check("the wrapped budget payload has a code")?,
        &"sync_budget_exhausted",
        "wrapped discovery budget failure carries the sync_budget_exhausted code",
    )?;

    let raced = map_publish_error(
        root,
        &publish::PublishError::ChangedDuringSample {
            path: "src/lib.rs".to_owned(),
        },
        "test",
    );
    ensure_eq(
        raced
            .data
            .check("the CAS race carries a payload")?
            .get("code")
            .check("the CAS race payload has a code")?,
        &"stale_revision",
        "CAS race failure carries the stale_revision code",
    )?;

    let other = map_publish_error(
        root,
        &publish::PublishError::InvalidParserPackVersion("nope".to_owned()),
        "test",
    );
    ensure(
        other.data.is_none(),
        "a failure no caller can act on differently must not carry a code",
    )?;
    Ok(())
}

/// Asserts that `ensure_project_row` returned `ProjectMetaError::Other`
/// with the expected context prefix and underlying cause substring.
fn assert_other_meta_error(
    result: Result<(), super::ProjectMetaError>,
    expected_prefix: &str,
    expected_substring: &str,
) -> TestResult {
    let error = result.check_err("ensure_project_row must fail")?;
    let super::ProjectMetaError::Other(error) = error else {
        return Err(format!("expected ProjectMetaError::Other, got {error:?}"))
            .check("unexpected outcome");
    };
    ensure(
        error.message.starts_with(expected_prefix),
        format!("got: {}", error.message),
    )?;
    ensure(
        error.message.contains(expected_substring),
        format!("the source error must be preserved: {}", error.message),
    )?;
    Ok(())
}

/// A missing `project` table causes the initial `SELECT` to fail closed
/// immediately and report the reading arm, rather than swallowing the error
/// into `INSERT`.
#[test]
fn a_missing_project_table_surfaces_the_reading_error() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
    connection
        .execute_batch("DROP TABLE project")
        .check("drop the project table")?;

    assert_other_meta_error(
        ensure_project_row(&mut connection, Path::new("/tmp/example-project")),
        "reading project metadata: ",
        "no such table: project",
    )
}

/// When the `project` table exists and is empty (`Ok(None)` on initial
/// `SELECT`), a denied `INSERT` is reported as the recording arm.
#[test]
fn a_failed_metadata_write_surfaces_the_recording_error() -> TestResult {
    let (_dir, mut connection) = open_temp_db()?;
    connection.authorizer(Some(
        |context: rusqlite::hooks::AuthContext<'_>| match context.action {
            rusqlite::hooks::AuthAction::Insert {
                table_name: "project",
            } => rusqlite::hooks::Authorization::Deny,
            _ => rusqlite::hooks::Authorization::Allow,
        },
    ));

    assert_other_meta_error(
        ensure_project_row(&mut connection, Path::new("/tmp/example-project")),
        "recording project metadata: ",
        "not authorized",
    )
}

/// A failed metadata `SELECT` must be reported as the reading arm when the
/// authorizer denies reading `project.root_path`.
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

    assert_other_meta_error(
        ensure_project_row(&mut connection, Path::new("/tmp/example-project")),
        "reading project metadata: ",
        "access to project.root_path is prohibited",
    )
}

/// A corrupt SQLite database surfaces `ProjectMetaError::Corrupt` from
/// `ensure_project_row` so the orchestrator can discard and rebuild it.
#[test]
fn a_corrupt_database_surfaces_the_corrupt_variant() -> TestResult {
    let dir = tempfile::tempdir().check("temp dir")?;
    let db_path = dir.path().join("corrupt.db");
    std::fs::write(&db_path, b"this is not a sqlite database").check("write corrupt db")?;
    let mut connection = rusqlite::Connection::open(&db_path).check("open corrupt db")?;

    let error = ensure_project_row(&mut connection, Path::new("/tmp/example-project"))
        .check_err("a corrupt database must fail ensure_project_row")?;

    let super::ProjectMetaError::Corrupt(sqlite_error) = error else {
        return Err(format!("expected ProjectMetaError::Corrupt, got {error:?}"))
            .check("unexpected outcome");
    };
    ensure(
        store::is_rusqlite_corruption(&sqlite_error),
        format!("expected SQLite corruption error, got {sqlite_error:?}"),
    )?;
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

    ensure(
        error
            .message
            .starts_with("recreating the project database: "),
        format!("got: {}", error.message),
    )?;
    ensure(
        error.message.contains("failed to open database"),
        format!(
            "the store's open failure must be preserved: {}",
            error.message
        ),
    )?;
    Ok(())
}

/// One failed manager-path publish is logged once, by
/// `server_runner::log_completion` when the tool call returns it: neither
/// `publish` nor `map_publish_error` logs it at warn first. The returned
/// message carries the context those sites used to log.
#[test]
fn a_failed_manager_publish_is_returned_with_its_context_and_not_logged_first() -> TestResult {
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
        .map_err(|error| map_publish_error(project.path(), &error, "sync"))
    })?;

    ensure(
        logs.contains("once-probe-capture-control"),
        format!("the capture must be live: {logs}"),
    )?;
    let warnings = logs
        .lines()
        .filter(|line| line.contains("WARN") && !line.contains("once-probe-capture-control"))
        .count();
    ensure_eq(
        &warnings,
        &0,
        format!("a returned failure is not logged first: {logs}"),
    )?;
    let error = result.err().check("the trigger must fail the publish")?;
    ensure(
        error.message.contains("injected publish failure"),
        format!("the cause is kept: {}", error.message),
    )?;
    ensure(
        error
            .message
            .contains(&format!("root: {}", project.path().display())),
        format!("the root is named: {}", error.message),
    )?;
    ensure(
        error.message.contains("error_kind: "),
        format!("the category is named: {}", error.message),
    )?;
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

    ensure(
        error.message.contains("turned off during this call"),
        format!("{}", error.message),
    )?;
    ensure_eq(
        &error.data.as_ref().and_then(|data| data.get("code")),
        &Some(&serde_json::json!("project_not_enabled")),
        format!("{error:?}"),
    )?;
    ensure(!activation.exists(), "the directory is not recreated")?;
    Ok(())
}

#[test]
fn map_publish_error_attaches_stale_revision_for_stale_baseline() -> TestResult {
    let error = crate::sync::publish::PublishError::Revision(
        crate::sync::revision::RevisionError::StaleBaseline {
            expected: "rev-1".to_owned(),
            found: "rev-2".to_owned(),
        },
    );
    let mapped = map_publish_error(Path::new("/tmp/project"), &error, "sync");
    ensure_eq(
        &mapped.data,
        &Some(crate::error_codes::payload(
            crate::error_codes::ErrorCode::StaleRevision,
        )),
        "StaleBaseline maps to StaleRevision error code",
    )?;
    Ok(())
}
