//! Skip collection lives beside the diff that consumes it; these tests
//! stay with the publish scenarios that own the "logged once" contract.

use crate::store::test_capture::capture_warns;
use crate::sync::discovery;
use crate::sync::publish_diff::collect_skipped;
use crate::sync::revision::SKIP_REASON_STAT_FAILED;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::collections::{BTreeMap, HashSet};

fn skipped(path: std::path::PathBuf, code: &'static str) -> discovery::SkippedFile {
    discovery::SkippedFile {
        absolute_path: path,
        reason: "read failed".to_owned(),
        code,
    }
}

#[test]
fn a_stat_failed_skip_logs_exactly_one_warn_line() -> TestResult {
    let root = tempfile::tempdir().check("tempdir")?;
    // A regular file used as a directory: the stat fails with ENOTDIR,
    // not NotFound, so the skip is recorded rather than retried.
    std::fs::write(root.path().join("plain.rs"), b"fn plain() {}\n").check("write file")?;
    let files = [skipped(
        root.path().join("plain.rs").join("inner.rs"),
        "unreadable",
    )];
    let mut paths = HashSet::new();
    let mut records = BTreeMap::new();

    let (result, logs) =
        capture_warns(|| collect_skipped(root.path(), &files, None, &mut paths, &mut records))?;
    result.check("collect the skip")?;

    let lines = logs
        .lines()
        .filter(|line| line.contains("stat failed for a skipped file"))
        .count();
    ensure_eq(&lines, &1, format!("one warn per stat-failed skip: {logs}"))?;
    let record = records
        .get("plain.rs/inner.rs")
        .check("the skip has a record")?;
    ensure_eq(
        &record.reason,
        &SKIP_REASON_STAT_FAILED,
        "record.reason == SKIP_REASON_STAT_FAILED",
    )?;
    ensure_eq(&record.byte_len, &None, "record.byte_len == None")?;
    Ok(())
}

#[test]
fn a_skip_deleted_since_the_walk_fails_the_attempt_as_a_tree_change() -> TestResult {
    let root = tempfile::tempdir().check("tempdir")?;
    let files = [skipped(root.path().join("vanished.rs"), "unreadable")];
    let mut paths = HashSet::new();
    let mut records = BTreeMap::new();

    let error = collect_skipped(root.path(), &files, None, &mut paths, &mut records)
        .check_err("a vanished skip must not be recorded")?;

    ensure(
        matches!(
            &error,
            crate::sync::PublishError::ChangedDuringSample { path } if path == "vanished.rs"
        ),
        format!("got {error:?}"),
    )?;
    ensure(records.is_empty(), "records.is_empty()")?;
    Ok(())
}

#[test]
fn a_skip_whose_stat_succeeds_keeps_the_publish_loop_warn() -> TestResult {
    let root = tempfile::tempdir().check("tempdir")?;
    let present = root.path().join("present.rs");
    std::fs::write(&present, b"fn a() {}").check("write file")?;
    let files = [skipped(present, "unreadable")];
    let mut paths = HashSet::new();
    let mut records = BTreeMap::new();

    let (result, logs) =
        capture_warns(|| collect_skipped(root.path(), &files, None, &mut paths, &mut records))?;
    result.check("collect the skip")?;

    let lines = logs
        .lines()
        .filter(|line| line.contains("skipped file during unreadable"))
        .count();
    ensure_eq(&lines, &1, &logs)?;
    ensure_eq(
        &records
            .get("present.rs")
            .check("a record for present.rs")?
            .reason,
        &"unreadable",
        "records[\"present.rs\"].reason == \"unreadable\"",
    )?;
    Ok(())
}
