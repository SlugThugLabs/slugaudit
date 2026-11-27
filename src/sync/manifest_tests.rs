use super::*;
use slugaudit_test_check::{TestResult, ensure, ensure_eq};

#[test]
fn unchanged_file_produces_no_change() -> TestResult {
    let changes = compare([("a.rs", "hash1")], [("a.rs", "hash1")]);
    ensure(changes.is_empty(), "changes.is_empty()")?;
    Ok(())
}

#[test]
fn same_size_content_change_is_modified() -> TestResult {
    let changes = compare([("a.rs", "hash1")], [("a.rs", "hash2")]);
    ensure_eq(
        &changes,
        &vec![FileChange {
            relative_path: "a.rs".to_owned(),
            status: ChangeStatus::Modified,
        }],
        "changes == vec![FileChange { relative_path: \"a.rs\".to_owned(), status: ChangeStatus::Modified, }]",
    )?;
    Ok(())
}

#[test]
fn new_file_is_added() -> TestResult {
    let changes = compare([], [("a.rs", "hash1")]);
    ensure_eq(
        &changes,
        &vec![FileChange {
            relative_path: "a.rs".to_owned(),
            status: ChangeStatus::Added,
        }],
        "changes == vec![FileChange { relative_path: \"a.rs\".to_owned(), status: ChangeStatus::Added, }]",
    )?;
    Ok(())
}

#[test]
fn missing_file_is_deleted() -> TestResult {
    let changes = compare([("a.rs", "hash1")], []);
    ensure_eq(
        &changes,
        &vec![FileChange {
            relative_path: "a.rs".to_owned(),
            status: ChangeStatus::Deleted,
        }],
        "changes == vec![FileChange { relative_path: \"a.rs\".to_owned(), status: ChangeStatus::Deleted, }]",
    )?;
    Ok(())
}

#[test]
fn mixed_changes_are_all_reported_and_sorted() -> TestResult {
    let changes = compare(
        [
            ("deleted.rs", "h1"),
            ("modified.rs", "h2"),
            ("same.rs", "h3"),
        ],
        [("added.rs", "h4"), ("modified.rs", "h5"), ("same.rs", "h3")],
    );
    ensure_eq(
        &changes,
        &vec![
            FileChange {
                relative_path: "added.rs".to_owned(),
                status: ChangeStatus::Added,
            },
            FileChange {
                relative_path: "deleted.rs".to_owned(),
                status: ChangeStatus::Deleted,
            },
            FileChange {
                relative_path: "modified.rs".to_owned(),
                status: ChangeStatus::Modified,
            },
        ],
        "mixed changes sorted by relative_path",
    )?;
    Ok(())
}
