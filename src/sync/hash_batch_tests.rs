//! Tests for the Pass 1 single-open binary sniff + stream hashing pass.

use super::*;
use crate::sync::discovery::FileKind;
use crate::sync::test_support::write;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn walked(project: &std::path::Path, relative_path: &str) -> WalkedFile {
    WalkedFile {
        relative_path: relative_path.to_owned(),
        absolute_path: project.join(relative_path),
    }
}

#[test]
fn hashing_an_empty_file_list_returns_empty_results() -> TestResult {
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let out = sniff_and_hash_walked(&[], 1024, &deadline).check("empty hash")?;
    ensure(out.discovered.is_empty(), "empty input yields no files")?;
    ensure(
        out.identities.is_empty(),
        "empty input yields no identities",
    )?;
    ensure(out.skips.is_empty(), "empty input yields no skips")?;
    Ok(())
}

#[test]
fn hashing_normal_and_binary_files_preserves_walk_order_and_kinds() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "a.rs", b"fn a() {}\n")?;
    write(project.path(), "b.bin", &[0x00, 0x01, 0x02])?;

    let entries = vec![
        walked(project.path(), "a.rs"),
        walked(project.path(), "b.bin"),
    ];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let out = sniff_and_hash_walked(&entries, 1024, &deadline).check("hash batch")?;

    let kinds: Vec<(&str, FileKind)> = out
        .discovered
        .iter()
        .map(|file| (file.relative_path.as_str(), file.kind))
        .collect();
    ensure_eq(
        &kinds,
        &[("a.rs", FileKind::Indexed), ("b.bin", FileKind::Binary)],
        "Pass 1 preserves order and classifies text vs binary in one open",
    )?;
    let paths: Vec<&str> = out
        .identities
        .iter()
        .map(|identity| identity.relative_path.as_str())
        .collect();
    ensure_eq(&paths, &["a.rs", "b.bin"], "identities preserve walk order")?;
    ensure(out.skips.is_empty(), "normal files produce no skips")?;
    Ok(())
}

#[test]
fn an_oversized_file_is_skipped_with_the_hash_error_message() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "big.rs", &[b'x'; 20])?;

    let entries = vec![walked(project.path(), "big.rs")];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let out = sniff_and_hash_walked(&entries, 10, &deadline).check("hash batch")?;

    ensure(
        out.identities.is_empty(),
        "oversized file produces no hashed identity",
    )?;
    let [skip] = out.skips.as_slice() else {
        return Err(format!("expected one skip, got {:?}", out.skips)).check("skips");
    };
    ensure_eq(
        &skip.code,
        &SKIP_REASON_OVERSIZE,
        "oversized file records oversize skip code",
    )?;
    ensure_eq(
        &skip.absolute_path,
        &project.path().join("big.rs"),
        "skip carries absolute path of oversized file",
    )?;
    ensure_eq(
        &skip.reason,
        &format!(
            "{} exceeds the 10-byte per-file hash ceiling; at least 20 bytes were read \
             before the cap stopped the read",
            project.path().join("big.rs").display()
        ),
        "skip reason reports per-file hash ceiling and lower-bound bytes read",
    )?;
    Ok(())
}

#[test]
fn an_expired_deadline_aborts_with_time_budget_exceeded_naming_the_hashing() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "slow.rs", b"content\n")?;

    let entries = vec![walked(project.path(), "slow.rs")];
    let deadline = Deadline::after(std::time::Duration::ZERO);
    let error = sniff_and_hash_walked(&entries, 1024, &deadline).check_err("deadline must trip")?;

    ensure(
        matches!(
            &error,
            PublishError::TimeBudgetExceeded {
                activity: "hashing",
                path: Some(p),
                ..
            } if p == "slow.rs"
        ),
        format!("got {error:?}"),
    )?;
    ensure(
        error.to_string().contains("while hashing slow.rs"),
        format!("display message names hashing and slow.rs: {error}"),
    )?;
    Ok(())
}

#[test]
fn a_file_deleted_since_the_walk_is_a_change_the_hash_pass_reports() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "keep.rs", b"fn keep() {}\n")?;

    let entries = vec![
        walked(project.path(), "gone.rs"),
        walked(project.path(), "keep.rs"),
    ];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let error = sniff_and_hash_walked(&entries, 1024, &deadline)
        .check_err("a vanished file must fail the pass")?;

    ensure(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "gone.rs"),
        format!("got {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_non_regular_entry_in_the_hash_pass_is_skipped_as_unreadable() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    std::fs::create_dir(project.path().join("dir.rs")).check("create a directory")?;

    let entries = vec![walked(project.path(), "dir.rs")];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let out = sniff_and_hash_walked(&entries, 1024, &deadline)
        .check("non-regular file is skipped as unreadable")?;

    ensure(out.discovered.is_empty(), "directory is not discovered")?;
    let [skip] = out.skips.as_slice() else {
        return Err(format!("expected one skip, got {:?}", out.skips)).check("skips");
    };
    ensure_eq(
        &skip.code,
        &SKIP_REASON_UNREADABLE,
        "non-regular file records unreadable skip code",
    )?;
    Ok(())
}
