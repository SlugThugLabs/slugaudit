//! The `.gitignore` guard `enable` adds: the rule lands once, every existing
//! byte and the file's line-ending convention are kept, and a file that
//! cannot be read, written or fully inspected is reported, never fatal.
//! The writer's own failure arms are in `gitignore_tests.rs`.

use super::*;

/// Resolves `directory` and writes `contents` to its root `.gitignore`,
/// returning both the root and the file's path.
fn gitignore_fixture(
    directory: &tempfile::TempDir,
    contents: &[u8],
) -> TestResult<(ProjectRoot, PathBuf)> {
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let path = root.as_path().join(GITIGNORE_FILE);
    fs::write(&path, contents).check("write .gitignore")?;
    Ok((root, path))
}

#[test]
fn a_project_without_a_gitignore_gains_the_rule_in_a_new_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let path = root.as_path().join(GITIGNORE_FILE);

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        b".planning/slugaudit/\n".to_vec(),
        "the created file holds exactly the one rule"
    );
    Ok(())
}

#[test]
fn appending_keeps_every_existing_byte_of_the_gitignore() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let original = b"/target\n*.log\n";
    let (root, path) = gitignore_fixture(&directory, original)?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    let written = fs::read(&path).check("read .gitignore")?;
    assert!(
        written.starts_with(original),
        "the user's bytes must come through unchanged: {written:?}"
    );
    assert_eq!(&written[original.len()..], b".planning/slugaudit/\n");
    Ok(())
}

#[test]
fn an_equivalent_rule_is_recognised_and_not_duplicated() -> TestResult {
    for rule in [
        ".planning/slugaudit/",
        ".planning/slugaudit",
        "/.planning/slugaudit/",
        "/.planning/slugaudit",
    ] {
        let directory = tempfile::tempdir().check("temp dir")?;
        let contents = format!("{rule}\n");
        let (root, path) = gitignore_fixture(&directory, contents.as_bytes())?;

        let outcome = ensure_gitignore(&root);

        assert_eq!(
            outcome,
            GitignoreOutcome::AlreadyIgnored(path.clone()),
            "rule {rule:?} already ignores the directory"
        );
        assert_eq!(
            fs::read(&path).check("read .gitignore")?,
            contents.as_bytes(),
            "an already-ignored project's file stays byte-identical"
        );
    }
    Ok(())
}

#[test]
fn a_commented_or_negated_line_does_not_make_the_directory_ignored() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let contents = b"# .planning/slugaudit/\n!.planning/slugaudit/\n.planning/slugaudit-extra/\n";
    let (root, path) = gitignore_fixture(&directory, contents)?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert!(
        fs::read(&path)
            .check("read .gitignore")?
            .ends_with(b"\n.planning/slugaudit/\n"),
        "the real rule is appended after the decoys"
    );
    Ok(())
}

#[test]
fn a_last_line_without_a_newline_is_not_merged_with_the_rule() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let (root, path) = gitignore_fixture(&directory, b"/target")?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        b"/target\n.planning/slugaudit/\n".to_vec(),
        "the user's last line keeps its own line"
    );
    Ok(())
}

#[test]
fn an_empty_gitignore_file_gains_only_the_rule() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let (root, path) = gitignore_fixture(&directory, b"")?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        b".planning/slugaudit/\n".to_vec(),
        "no blank line is introduced ahead of the rule"
    );
    Ok(())
}

#[test]
fn a_crlf_gitignore_keeps_its_line_ending_convention() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let (root, path) = gitignore_fixture(&directory, b"/target\r\n")?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        b"/target\r\n.planning/slugaudit/\r\n".to_vec(),
        "the appended rule uses CRLF, not a mixed ending"
    );
    Ok(())
}

#[test]
fn a_crlf_gitignore_without_a_final_newline_gets_a_crlf_separator() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let (root, path) = gitignore_fixture(&directory, b"/a\r\n/b")?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        b"/a\r\n/b\r\n.planning/slugaudit/\r\n".to_vec(),
        "the separator matches the file's convention too"
    );
    Ok(())
}

#[test]
fn an_unreadable_gitignore_is_reported_rather_than_fatal() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let path = root.as_path().join(GITIGNORE_FILE);
    fs::create_dir(&path).check("a directory where the ignore file belongs")?;

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the unwritable path is named");
            assert!(!reason.is_empty(), "the reason is reported");
        }
        other => {
            return Err(format!(
                "a file that cannot be read must report NotWritten, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    Ok(())
}

/// `NotWritten` promises the file was left untouched. The write is atomic
/// (temp file in the same directory, then rename), so even a failure after
/// the read cannot have created the path or appended a partial rule — and
/// no temp file may survive the failure.
#[test]
fn a_failed_gitignore_write_leaves_the_original_untouched() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("valid root")?;
    let path = root.as_path().join(GITIGNORE_FILE);
    fs::create_dir(&path).check("a directory where the ignore file belongs")?;
    let marker = path.join("keep");
    fs::write(&marker, b"x").check("write a marker inside the directory")?;

    let outcome = ensure_gitignore(&root);

    assert!(matches!(outcome, GitignoreOutcome::NotWritten { .. }));
    assert!(
        path.is_dir(),
        "the original path is still the same directory"
    );
    assert_eq!(
        fs::read(&marker).check("read the marker")?,
        b"x",
        "nothing inside the original path was touched"
    );
    let mut leftovers = 0;
    for entry in fs::read_dir(root.as_path()).check("read the project root")? {
        let entry = entry.check("read a project root entry")?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".gitignore.slugaudit.")
        {
            leftovers += 1;
        }
    }
    assert_eq!(leftovers, 0, "no temp file survives a failed write");
    Ok(())
}

#[test]
fn an_oversized_gitignore_is_reported_and_left_byte_identical() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let cap = usize::try_from(MAX_FOREIGN_TEXT_BYTES).check("cap fits usize")?;
    let contents = vec![b'#'; cap + 1];
    let (root, path) = gitignore_fixture(&directory, &contents)?;

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the over-cap path is named");
            assert!(
                reason.contains(&format!("{MAX_FOREIGN_TEXT_BYTES}-byte cap")),
                "the reason names the cap rather than merely failing: {reason}"
            );
        }
        other => {
            return Err(format!(
                "an over-cap .gitignore must be reported, not read, got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        contents,
        "an over-cap file is never truncated or appended to"
    );
    Ok(())
}

#[test]
fn a_gitignore_exactly_at_the_cap_is_still_inspected() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let cap = usize::try_from(MAX_FOREIGN_TEXT_BYTES).check("cap fits usize")?;
    let rule = b".planning/slugaudit/\n";
    let mut contents = vec![b'\n'; cap - rule.len()];
    contents.extend_from_slice(rule);
    assert_eq!(contents.len(), cap, "the fixture sits exactly on the cap");
    let (root, path) = gitignore_fixture(&directory, &contents)?;

    let outcome = ensure_gitignore(&root);

    assert_eq!(
        outcome,
        GitignoreOutcome::AlreadyIgnored(path.clone()),
        "a file exactly at the cap is inspected, not rejected"
    );
    assert_eq!(
        fs::read(&path).check("read .gitignore")?,
        contents,
        "inspecting the file leaves it byte-identical"
    );
    Ok(())
}
