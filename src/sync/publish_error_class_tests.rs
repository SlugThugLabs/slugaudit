use super::*;

#[test]
fn publish_error_corruption_classification_matches_sqlite_corruption() -> TestResult {
    let corrupt_db = PublishError::Database(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ffi::ErrorCode::DatabaseCorrupt,
            extended_code: 11,
        },
        Some("database disk image is malformed".into()),
    ));
    ensure(corrupt_db.is_corruption(), "corrupt_db.is_corruption()")?;

    let corrupt_rev =
        PublishError::Revision(RevisionError::Database(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::DatabaseCorrupt,
                extended_code: 11,
            },
            None,
        )));
    ensure(corrupt_rev.is_corruption(), "corrupt_rev.is_corruption()")?;

    ensure(
        !PublishError::Database(rusqlite::Error::QueryReturnedNoRows).is_corruption(),
        "!PublishError::Database(rusqlite::Error::QueryReturnedNoRows).is_corruption()",
    )?;
    ensure(
        !PublishError::Revision(RevisionError::Database(
            rusqlite::Error::QueryReturnedNoRows,
        ))
        .is_corruption(),
        "!PublishError::Revision(RevisionError::Database( rusqlite::Error::QueryReturnedNoRows )) .is_corruption()",
    )?;
    ensure(
        !PublishError::Revision(RevisionError::StaleBaseline {
            expected: "rev-1".into(),
            found: "rev-2".into(),
        })
        .is_corruption(),
        "!PublishError::Revision(RevisionError::StaleBaseline { expected: \"rev-1\".into(), found: \"rev-2\".into(), }) .is_corruption()",
    )?;
    ensure(
        !PublishError::TimeBudgetExceeded {
            path: String::new(),
            elapsed_ms: 100,
        }
        .is_corruption(),
        "!PublishError::TimeBudgetExceeded { path: String::new(), elapsed_ms: 100, } .is_corruption()",
    )?;
    ensure(
        !PublishError::SamplingResultLost {
            path: "a.rs".into(),
            reason: "channel closed".into(),
        }
        .is_corruption(),
        "!PublishError::SamplingResultLost { path: \"a.rs\".into(), reason: \"channel closed\".into(), } .is_corruption()",
    )?;
    Ok(())
}

#[test]
fn publish_error_budget_elapsed() -> TestResult {
    let budget_pub = PublishError::TimeBudgetExceeded {
        path: " a.rs".into(),
        elapsed_ms: 42,
    };
    let budget_disc =
        PublishError::Discovery(discovery::DiscoveryError::TimeBudgetExceeded { elapsed_ms: 84 });
    let changed = PublishError::ChangedDuringSample {
        path: "changed.rs".into(),
    };
    ensure_eq(
        &budget_pub.budget_elapsed_ms(),
        &Some(42),
        "budget_pub.budget_elapsed_ms() == Some(42)",
    )?;
    ensure_eq(
        &budget_disc.budget_elapsed_ms(),
        &Some(84),
        "budget_disc.budget_elapsed_ms() == Some(84)",
    )?;
    ensure_eq(
        &changed.budget_elapsed_ms(),
        &None,
        "changed.budget_elapsed_ms() == None",
    )?;
    Ok(())
}

#[test]
fn a_file_that_vanished_during_discovery_becomes_the_retryable_tree_change() -> TestResult {
    let error = PublishError::from(discovery::DiscoveryError::Vanished {
        relative_path: "gone.rs".into(),
    });

    ensure(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "gone.rs"),
        format!("got {error:?}"),
    )?;
    ensure(is_retryable(&error), "is_retryable(&error)")?;
    Ok(())
}

#[test]
fn a_read_after_the_walk_is_a_tree_change_only_when_the_file_is_gone() -> TestResult {
    let gone = PublishError::read_after_walk(
        "gone.rs",
        std::path::PathBuf::from("/p/gone.rs"),
        std::io::Error::from(std::io::ErrorKind::NotFound),
    );
    let denied = PublishError::read_after_walk(
        "denied.rs",
        std::path::PathBuf::from("/p/denied.rs"),
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    );

    ensure(
        matches!(&gone, PublishError::ChangedDuringSample { path } if path == "gone.rs"),
        format!("got {gone:?}"),
    )?;
    ensure(
        matches!(&denied, PublishError::Sample(sample::SampleError::Read { path, source })
            if *path == std::path::Path::new("/p/denied.rs")
                && source.kind() == std::io::ErrorKind::PermissionDenied),
        format!("got {denied:?}"),
    )?;
    ensure(!is_retryable(&denied), "!is_retryable(&denied)")?;
    Ok(())
}

#[test]
fn unrepresentable_paths_storage_and_replacement_roundtrip() -> TestResult {
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    let initial = stored_unrepresentable_paths(&connection).check("read empty stored")?;
    ensure(initial.is_empty(), "initial.is_empty()")?;

    let mut set_a = HashSet::new();
    set_a.insert(vec![0xFF, 0xFE]);
    replace_unrepresentable_paths(&mut connection, &set_a).check("insert unrepresentable")?;
    ensure_eq(
        &stored_unrepresentable_paths(&connection).check("read unrepresentable")?,
        &set_a,
        "stored_unrepresentable_paths(&connection).check(\"read unrepresentable\")? == set_a",
    )?;

    replace_unrepresentable_paths(&mut connection, &set_a).check("no-op unrepresentable")?;
    ensure_eq(
        &stored_unrepresentable_paths(&connection).check("read unrepresentable")?,
        &set_a,
        "stored_unrepresentable_paths(&connection).check(\"read unrepresentable\")? == set_a",
    )?;

    let empty = HashSet::new();
    replace_unrepresentable_paths(&mut connection, &empty).check("clear unrepresentable")?;
    ensure(
        stored_unrepresentable_paths(&connection)
            .check("read cleared")?
            .is_empty(),
        "stored_unrepresentable_paths(&connection) .check(\"read cleared\")? .is_empty()",
    )?;
    Ok(())
}
