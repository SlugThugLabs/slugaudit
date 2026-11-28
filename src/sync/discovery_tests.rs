use super::*;
use crate::sync::publish::PublishError;
use crate::sync::test_support::write;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::fs;

#[test]
fn discovers_source_config_and_doc_files_in_sorted_order() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, "src/main.rs", b"fn main() {}")?;
    write(root, "README.md", b"# Title")?;
    write(root, ".github/workflows/ci.yml", b"name: ci")?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    ensure_eq(
        &paths,
        &vec![".github/workflows/ci.yml", "README.md", "src/main.rs"],
        "paths == vec![\".github/workflows/ci.yml\", \"README.md\", \"src/main.rs\"]",
    )?;
    Ok(())
}

#[test]
fn classifies_binary_files_without_dropping_them() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(
        root,
        "image.png",
        &[0x89, b'P', b'N', b'G', 0x00, 0x01, 0x02],
    )?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    ensure_eq(&files.len(), &1, "files.len() == 1")?;
    ensure_eq(
        &files.first().check("files[0] exists")?.kind,
        &FileKind::Binary,
        "files[0].kind == FileKind::Binary",
    )?;
    Ok(())
}

#[test]
fn excludes_the_activation_directory() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, "src/lib.rs", b"pub fn lib() {}")?;
    write(root, ".planning/slugaudit/project.db", b"sqlite-bytes")?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    ensure_eq(&paths, &vec!["src/lib.rs"], "paths == vec![\"src/lib.rs\"]")?;
    Ok(())
}

#[test]
fn excludes_planning_and_target_directories() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, ".planning/slugaudit/project.db", b"sqlite-bytes")?;
    write(root, ".planning/README.md", b"# readme")?;
    write(root, "target/debug/out", b"bin")?;
    write(root, "planning/notes.md", b"# notes")?;
    write(root, "slugaudit/x.md", b"x")?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    ensure_eq(
        &paths,
        &vec!["planning/notes.md", "slugaudit/x.md"],
        "paths == vec![\"planning/notes.md\", \"slugaudit/x.md\"]",
    )?;
    Ok(())
}

#[test]
fn excludes_git_internals() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, "src/lib.rs", b"pub fn lib() {}")?;
    write(root, ".git/HEAD", b"ref: refs/heads/main")?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    ensure_eq(&paths, &vec!["src/lib.rs"], "paths == vec![\"src/lib.rs\"]")?;
    Ok(())
}

#[test]
fn honors_gitignore_content() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, ".gitignore", b"ignored.txt\n")?;
    write(root, "ignored.txt", b"should not appear")?;
    write(root, "kept.txt", b"should appear")?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    ensure_eq(
        &paths,
        &vec![".gitignore", "kept.txt"],
        "paths == vec![\".gitignore\", \"kept.txt\"]",
    )?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_non_utf8_filename_is_skipped_not_a_panic_or_a_whole_walk_failure() -> TestResult {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    let bad_name = OsString::from_vec(vec![b'b', b'a', 0xFF, b'd', b'.', b't', b'x', b't']);
    std::fs::write(root.join(&bad_name), b"content").check("write non-UTF-8-named file")?;
    std::fs::write(root.join("good.txt"), b"content").check("write a normal file")?;

    let (files, skipped) =
        discover_and_hash(root).check("a bad filename must not fail the whole walk")?;
    ensure_eq(
        &files.len(),
        &1,
        "the good file must still be discovered despite the bad one",
    )?;
    ensure_eq(
        &skipped.len(),
        &1,
        "the bad filename must be recorded as skipped",
    )?;
    let skipped_file = skipped.first().check("skipped[0] exists")?;
    ensure_eq(
        &skipped_file.absolute_path,
        &root.join(&bad_name),
        "the skip must name the file that was left out",
    )?;
    ensure_eq(
        &skipped_file.reason,
        &DiscoveryError::NonUtf8Path(root.join(&bad_name)).to_string(),
        "the skip reason must be the typed non-UTF-8 reason, not a generic one",
    )?;
    Ok(())
}

#[test]
fn a_path_outside_the_root_is_skipped_with_the_not_relative_reason() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let outside = tempfile::tempdir().check("outside dir")?;
    let outside_file = outside.path().join("escaped.rs");

    let skipped = strip_project_prefix(directory.path(), &outside_file)
        .check_err("a path outside the root must be reported, not stripped")?;

    ensure_eq(
        &skipped.absolute_path,
        &outside_file,
        "the skip must carry the path that could not be made relative",
    )?;
    ensure_eq(
        &skipped.reason,
        &DiscoveryError::NotRelative(outside_file.clone()).to_string(),
        "the skip reason must be the typed not-relative reason",
    )?;
    ensure(
        skipped
            .reason
            .contains("is not relative to the project root"),
        format!(
            "the reason must explain why the file was left out: {}",
            skipped.reason
        ),
    )?;
    Ok(())
}

/// A regular file discovered in one sync can be replaced by a symlink before
/// the next sync's walk (e.g. a build step or package tool swapping a file for
/// a link). `follow_links(false)` means the walker reports the symlink's own
/// `lstat` file type rather than following it, so `is_file()` drops it without
/// reading the target.
#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_symlink_between_syncs_is_never_dereferenced() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, "lib.rs", b"pub fn a() {}\n")?;

    let (first, _skipped) = discover_and_hash(root).check("first discover")?;
    ensure_eq(
        &(first
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>()),
        &vec!["lib.rs"],
        "first discover finds lib.rs",
    )?;

    let outside = tempfile::tempdir().check("outside dir")?;
    let external_target = outside.path().join("target.bin");
    fs::write(&external_target, [0_u8, 1, 2]).check("write outside file")?;

    std::fs::remove_file(root.join("lib.rs")).check("remove the real file")?;
    std::os::unix::fs::symlink(&external_target, root.join("lib.rs"))
        .check("replace with a symlink")?;

    let (second, _skipped) =
        discover_and_hash(root).check("second discover does not error or panic")?;
    ensure_eq(
        &second,
        &Vec::new(),
        "a path replaced by a symlink is skipped without following the link",
    )?;
    Ok(())
}

#[test]
fn scratch_and_temp_files_are_excluded_from_discovery() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, "src/lib.rs", b"pub fn lib() {}")?;
    write(root, "scratch.py", b"print('scratch')")?;
    write(root, "session.tmp", b"temp data")?;
    write(root, "notes.bak", b"backup")?;
    write(root, "buffer.swp", b"swap")?;
    write(root, "file~", b"emacs backup")?;
    write(root, "output.claude_output.txt", b"claude session output")?;

    let (files, _skipped) = discover_and_hash(root).check("discover_and_hash")?;
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    ensure_eq(
        &paths,
        &vec!["src/lib.rs"],
        format!(
            "only the real source file should be discovered, scratch/temp files must be excluded, got: {paths:?}"
        ),
    )?;
    Ok(())
}

#[test]
fn a_missing_project_root_fails_discovery_with_the_walk_error() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let missing = directory.path().join("gone");

    let error = discover_and_hash(&missing).check_err("an unlistable root must fail discovery")?;

    ensure(
        matches!(error, PublishError::Discovery(DiscoveryError::Walk(_))),
        format!("the typed walk failure must survive, got {error:?}"),
    )?;
    ensure(
        error.to_string().contains("failed to walk project files"),
        format!("the failure must name the walk: {error}"),
    )?;
    Ok(())
}

#[test]
fn a_walk_error_is_returned_with_the_failing_subdirectory_not_the_root() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    let failing = root.join("private-dir");
    let error = ignore::Error::WithPath {
        path: failing.clone(),
        err: Box::new(ignore::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ))),
    };
    let mut entries = Vec::new();
    let mut skipped = Vec::new();

    let error = collect_walked_entry(root, Err(error), &mut entries, &mut skipped)
        .check_err("a walk error must fail discovery")?;

    match error {
        DiscoveryError::Walk(error) => ensure_eq(
            &crate::ignore_rules::ignore_error_path(&error),
            &Some(failing.as_path()),
            "the failure must name the directory the walker could not read",
        )?,
        other => {
            return Err(format!("expected DiscoveryError::Walk, got {other:?}"))
                .check("unexpected outcome");
        }
    }
    ensure(
        entries.is_empty() && skipped.is_empty(),
        format!("a walk error is not a per-file skip: entries={entries:?} skipped={skipped:?}"),
    )?;
    Ok(())
}

#[test]
fn a_malformed_gitignore_fails_discovery_instead_of_dropping_its_rules() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path();
    write(root, ".gitignore", b"[z-a]\n")?;
    write(root, "kept.rs", b"fn kept() {}")?;

    let error =
        discover_and_hash(root).check_err("an unapplied ignore file must fail discovery")?;

    match error {
        PublishError::Discovery(DiscoveryError::IgnoreRules { kind, path }) => {
            ensure_eq(
                &kind,
                &"glob",
                "the static category names the parse failure",
            )?;
            let path = path.check("the walker names the ignore file")?;
            ensure_eq(
                &path.file_name().and_then(|name| name.to_str()),
                &Some(".gitignore"),
                "path.file_name().and_then(|name| name.to_str()) == Some(\".gitignore\")",
            )?;
        }
        other => {
            return Err(format!(
                "expected PublishError::Discovery(DiscoveryError::IgnoreRules), got {other:?}"
            ))
            .check("unexpected outcome");
        }
    }
    Ok(())
}

#[test]
fn a_failed_walk_publishes_no_revision_and_deletes_no_stored_rows() -> TestResult {
    use crate::progress::NoopProgressSink;
    use crate::store::open_read_write;
    use crate::sync::publish::publish;
    use crate::sync::test_support::stored_paths;

    let base = tempfile::tempdir().check("base dir")?;
    let root = base.path().join("root");
    fs::create_dir_all(&root).check("create project root")?;
    write(&root, "a.rs", b"fn a() {}")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection =
        open_read_write(&db_dir.path().join("project.db")).check("open database")?;
    let first = publish(&mut connection, &root, "1.0", &NoopProgressSink)
        .check("initial publish succeeds")?;

    let moved = base.path().join("moved-away");
    fs::rename(&root, &moved).check("move the project root away")?;

    let error = publish(&mut connection, &root, "1.0", &NoopProgressSink)
        .check_err("an unlistable root must fail the publish")?;

    ensure(
        matches!(error, PublishError::Discovery(DiscoveryError::Walk(_))),
        format!("the publish must carry the typed discovery failure: {error:?}"),
    )?;
    ensure_eq(
        &stored_paths(&connection)?,
        &["a.rs"],
        "no stored row may be deleted by a partial walk",
    )?;
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .check("read the current revision")?;
    ensure_eq(
        &current,
        &first.revision_id,
        "no revision may be published from a partial walk",
    )?;
    Ok(())
}
