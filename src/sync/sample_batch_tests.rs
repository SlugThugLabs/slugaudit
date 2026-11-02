//! Tests for the Pass 2 sampling pass.

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

fn generous() -> Deadline {
    Deadline::after(std::time::Duration::from_mins(1))
}

#[test]
fn a_file_deleted_since_the_walk_is_a_change_the_sample_pass_reports() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "keep.rs", b"fn keep() {}\n")?;
    let discovered = [
        indexed(project.path(), "gone.rs"),
        indexed(project.path(), "keep.rs"),
    ];

    let error = sample_all_with_deadline(
        &discovered,
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &generous(),
    )
    .check_err("a vanished file must fail the pass")?;

    assert!(
        matches!(&error, PublishError::ChangedDuringSample { path } if path == "gone.rs"),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn a_file_the_sample_pass_cannot_read_fails_with_its_read_error() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    std::fs::create_dir(project.path().join("dir.rs")).check("create a directory")?;
    let discovered = [indexed(project.path(), "dir.rs")];

    let error = sample_all_with_deadline(
        &discovered,
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &generous(),
    )
    .check_err("a directory cannot be sampled")?;

    assert!(
        matches!(&error, PublishError::Sample(SampleError::Read { path, .. })
            if *path == project.path().join("dir.rs")),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn per_file_skips_are_returned_sorted_with_their_codes() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "b-not-utf8.rs", &[0xff, 0xfe, b'a'])?;
    write(project.path(), "a-big.rs", &[b'x'; 64])?;
    write(project.path(), "c-ok.rs", b"fn ok() {}\n")?;
    let discovered = [
        indexed(project.path(), "b-not-utf8.rs"),
        indexed(project.path(), "a-big.rs"),
        indexed(project.path(), "c-ok.rs"),
    ];
    let limits = ResourceLimits {
        max_file_bytes: 32,
        ..ResourceLimits::default()
    };

    let (samples, skipped) = sample_all_with_deadline(
        &discovered,
        &limits,
        &crate::progress::NoopProgressSink,
        &generous(),
    )
    .check("sample")?;

    let sampled_paths: Vec<&str> = samples.iter().map(|s| s.relative_path.as_str()).collect();
    assert_eq!(sampled_paths, ["c-ok.rs"]);
    let codes: Vec<(std::path::PathBuf, &str)> = skipped
        .iter()
        .map(|skip| (skip.absolute_path.clone(), skip.code))
        .collect();
    assert_eq!(
        codes,
        [
            (project.path().join("a-big.rs"), SKIP_REASON_OVERSIZE),
            (project.path().join("b-not-utf8.rs"), SKIP_REASON_NOT_UTF8),
        ]
    );
    Ok(())
}

#[test]
fn an_expired_deadline_names_the_file_being_processed() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "lib.rs", b"fn a() {}\n")?;
    let discovered = [indexed(project.path(), "lib.rs")];

    let error = sample_all_with_deadline(
        &discovered,
        &ResourceLimits::default(),
        &crate::progress::NoopProgressSink,
        &Deadline::after(std::time::Duration::ZERO),
    )
    .check_err("the deadline must trip")?;

    assert!(
        matches!(&error, PublishError::TimeBudgetExceeded { path, .. } if path == " while processing lib.rs"),
        "got {error:?}"
    );
    Ok(())
}

#[test]
fn claim_bytes_accepts_up_to_the_limit_and_refuses_past_it() -> TestResult {
    let total = AtomicU64::new(0);

    claim_bytes(&total, 6, 10).check("under the limit")?;
    claim_bytes(&total, 4, 10).check("exactly at the limit")?;
    let error = claim_bytes(&total, 1, 10).check_err("one byte past the limit")?;

    assert!(
        matches!(
            error,
            PublishError::ImportTooLarge {
                total: 11,
                limit: 10
            }
        ),
        "got {error:?}"
    );
    assert_eq!(
        total.load(Ordering::Acquire),
        10,
        "a refused claim adds nothing"
    );
    Ok(())
}
