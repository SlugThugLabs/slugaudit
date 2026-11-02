//! Tests for the Pass 1 hashing pass.

use super::*;
use crate::sync::discovery::FileKind;
use crate::sync::test_support::write;
use slugaudit_test_check::{Check, CheckErr, TestResult};

fn indexed(project: &std::path::Path, relative_path: &str) -> DiscoveredFile {
    DiscoveredFile {
        relative_path: relative_path.to_owned(),
        absolute_path: project.join(relative_path),
        kind: FileKind::Indexed,
    }
}

#[test]
fn hashing_an_empty_file_list_returns_empty_results() -> TestResult {
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let (identities, skips) = hash_all_with_deadline(&[], 1024, &deadline).check("empty hash")?;
    assert!(identities.is_empty());
    assert!(skips.is_empty());
    Ok(())
}

#[test]
fn hashing_normal_files_produces_identities_in_discovery_order() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "a.rs", b"fn a() {}\n")?;
    write(project.path(), "b.rs", b"fn b() {}\n")?;

    let discovered = vec![
        indexed(project.path(), "a.rs"),
        indexed(project.path(), "b.rs"),
    ];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let (identities, skips) =
        hash_all_with_deadline(&discovered, 1024, &deadline).check("hash batch")?;

    let paths: Vec<&str> = identities
        .iter()
        .map(|identity| identity.relative_path.as_str())
        .collect();
    assert_eq!(paths, ["a.rs", "b.rs"]);
    assert!(skips.is_empty());
    Ok(())
}

#[test]
fn an_oversized_file_is_skipped_with_the_hash_error_message() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "big.rs", &[b'x'; 20])?;

    let discovered = vec![indexed(project.path(), "big.rs")];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let (identities, skips) =
        hash_all_with_deadline(&discovered, 10, &deadline).check("hash batch")?;

    assert!(identities.is_empty());
    let [skip] = skips.as_slice() else {
        return Err(format!("expected one skip, got {skips:?}")).check("skips");
    };
    assert_eq!(skip.code, SKIP_REASON_OVERSIZE);
    assert_eq!(skip.absolute_path, project.path().join("big.rs"));
    // The bounded read stops at the cap + 1, so the size is a lower bound.
    assert_eq!(
        skip.reason,
        format!(
            "{} exceeds the 10-byte per-file hash ceiling; at least 11 bytes were read \
             before the cap stopped the read",
            project.path().join("big.rs").display()
        )
    );
    Ok(())
}

#[test]
fn an_expired_deadline_aborts_with_time_budget_exceeded_naming_the_hashing() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "slow.rs", b"content\n")?;

    let discovered = vec![indexed(project.path(), "slow.rs")];
    let deadline = Deadline::after(std::time::Duration::ZERO);
    let error =
        hash_all_with_deadline(&discovered, 1024, &deadline).check_err("deadline must trip")?;

    assert!(
        matches!(&error, PublishError::TimeBudgetExceeded { path, .. } if path == " while hashing slow.rs"),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn a_file_deleted_since_the_walk_is_a_change_the_hash_pass_reports() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    write(project.path(), "keep.rs", b"fn keep() {}\n")?;

    let discovered = vec![
        indexed(project.path(), "gone.rs"),
        indexed(project.path(), "keep.rs"),
    ];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let error = hash_all_with_deadline(&discovered, 1024, &deadline)
        .check_err("a vanished file must fail the pass")?;

    assert!(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "gone.rs"),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn a_file_the_hash_pass_cannot_read_fails_with_its_read_error() -> TestResult {
    let project = tempfile::tempdir().check("temp dir")?;
    std::fs::create_dir(project.path().join("dir.rs")).check("create a directory")?;

    let discovered = vec![indexed(project.path(), "dir.rs")];
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let error = hash_all_with_deadline(&discovered, 1024, &deadline)
        .check_err("a directory cannot be hashed")?;

    assert!(
        matches!(&error, PublishError::Sample(crate::sync::sample::SampleError::Read { path, .. })
            if *path == project.path().join("dir.rs")),
        "got {error:?}"
    );
    Ok(())
}
