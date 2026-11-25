//! Content repairs moving a file from unchanged to modified in the diff.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn diff_with_unchanged(unchanged: usize) -> publish_diff::Diff {
    publish_diff::Diff {
        changes: Vec::new(),
        manifest_hash: String::new(),
        added: 0,
        modified: 0,
        deleted: 0,
        unchanged,
    }
}

fn one_mismatch() -> (HashSet<String>, Vec<SourceIdentity>) {
    let identities = vec![SourceIdentity::new("a.rs".to_owned(), "h".to_owned())];
    (HashSet::from(["a.rs".to_owned()]), identities)
}

#[test]
fn a_repair_moves_one_file_from_unchanged_to_modified() -> TestResult {
    let (mismatched, identities) = one_mismatch();
    let mut diff = diff_with_unchanged(1);

    force_content_repairs(&mut diff, &mismatched, &identities, &[]).check("repair succeeds")?;

    ensure_eq(&diff.unchanged, &0, "unchanged count")?;
    ensure_eq(&diff.modified, &1, "modified count")?;
    Ok(())
}

#[test]
fn a_repair_also_covers_a_corrupted_skipped_row() -> TestResult {
    let mismatched = HashSet::from(["big.rs".to_owned()]);
    let skipped = vec![SkippedRecord {
        relative_path: "big.rs".to_owned(),
        reason: "oversize".to_owned(),
        identity: SourceIdentity::new("big.rs".to_owned(), "stat-h".to_owned()),
        byte_len: Some(999),
        modified_unix_seconds: None,
    }];
    let mut diff = diff_with_unchanged(1);

    force_content_repairs(&mut diff, &mismatched, &[], &skipped)
        .check("repair skipped succeeds")?;

    ensure_eq(&diff.unchanged, &0, "unchanged count")?;
    ensure_eq(&diff.modified, &1, "modified count")?;
    Ok(())
}

#[test]
fn a_repair_with_no_unchanged_file_left_is_a_named_error() -> TestResult {
    let (mismatched, identities) = one_mismatch();
    let mut diff = diff_with_unchanged(0);

    let error = force_content_repairs(&mut diff, &mismatched, &identities, &[])
        .check_err("no unchanged file to repair")?;

    ensure(
        matches!(&error, PublishError::UnchangedCountInconsistent { path } if path == "a.rs"),
        format!("got {error:?}"),
    )?;
    Ok(())
}
