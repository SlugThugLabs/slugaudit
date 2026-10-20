use super::*;
use crate::sync::test_support::write;
use std::fs;

#[test]
fn discovers_source_config_and_doc_files_in_sorted_order() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, "src/main.rs", b"fn main() {}");
    write(root, "README.md", b"# Title");
    write(root, ".github/workflows/ci.yml", b"name: ci");

    let (files, _skipped) = discover(root).expect("discover");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![".github/workflows/ci.yml", "README.md", "src/main.rs"]
    );
}

#[test]
fn classifies_binary_files_without_dropping_them() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(
        root,
        "image.png",
        &[0x89, b'P', b'N', b'G', 0x00, 0x01, 0x02],
    );

    let (files, _skipped) = discover(root).expect("discover");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].kind, FileKind::Binary);
}

#[test]
fn excludes_the_activation_directory() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, "src/lib.rs", b"pub fn lib() {}");
    write(root, ".planning/slugaudit/project.db", b"sqlite-bytes");

    let (files, _skipped) = discover(root).expect("discover");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs"]);
}

#[test]
fn indexes_a_planning_directory_that_only_shares_the_activation_prefix() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, ".planning/slugaudit/project.db", b"sqlite-bytes");
    write(root, ".planning/slugaudit-notes/x.md", b"# notes");
    write(root, ".planning/slugauditx", b"x");

    let (files, _skipped) = discover(root).expect("discover");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![".planning/slugaudit-notes/x.md", ".planning/slugauditx"]
    );
}

#[test]
fn excludes_git_internals() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, "src/lib.rs", b"pub fn lib() {}");
    write(root, ".git/HEAD", b"ref: refs/heads/main");

    let (files, _skipped) = discover(root).expect("discover");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs"]);
}

#[test]
fn honors_gitignore_content() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, ".gitignore", b"ignored.txt\n");
    write(root, "ignored.txt", b"should not appear");
    write(root, "kept.txt", b"should appear");

    let (files, _skipped) = discover(root).expect("discover");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    // .gitignore itself is a legitimate config file and stays indexed;
    // only the file it excludes, ignored.txt, is missing.
    assert_eq!(paths, vec![".gitignore", "kept.txt"]);
}

/// Relative paths are stored/queried as UTF-8 text, so a filename that
/// isn't valid UTF-8 must be skipped individually, with the reason
/// recorded, rather than panicking or failing the entire discovery walk
/// (a single bad filename anywhere in a project must not make every other
/// file in that project unqueryable). Unix filenames are just bytes (no
/// NUL, no `/`), so a lone `0xFF` byte is real and filesystem-legal here
/// even though it can never be valid UTF-8.
// APFS, macOS's default filesystem, refuses a non-UTF-8 file name outright
// (`EILSEQ`, "Illegal byte sequence"; verified on macos-latest CI 2026-10-16),
// so this input cannot arise there and the test cannot construct it.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_non_utf8_filename_is_skipped_not_a_panic_or_a_whole_walk_failure() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    let bad_name = OsString::from_vec(vec![b'b', b'a', 0xFF, b'd', b'.', b't', b'x', b't']);
    std::fs::write(root.join(&bad_name), b"content").expect("write non-UTF-8-named file");
    std::fs::write(root.join("good.txt"), b"content").expect("write a normal file");

    let (files, skipped) = discover(root).expect("a bad filename must not fail the whole walk");
    assert_eq!(
        files.len(),
        1,
        "the good file must still be discovered despite the bad one"
    );
    assert_eq!(
        skipped.len(),
        1,
        "the bad filename must be recorded as skipped"
    );
    let skipped_file = &skipped[0];
    assert_eq!(
        skipped_file.absolute_path,
        root.join(&bad_name),
        "the skip must name the file that was left out"
    );
    assert_eq!(
        skipped_file.reason,
        DiscoveryError::NonUtf8Path(root.join(&bad_name)).to_string(),
        "the skip reason must be the typed non-UTF-8 reason, not a generic one"
    );
}

/// The walker is expected to yield only paths beneath the root. If it ever
/// yields one outside it, the file must be reported with the typed
/// `NotRelative` reason rather than dropped silently; a path that cannot be
/// named relative to the project has no safe `files.path` value.
#[test]
fn a_path_outside_the_root_is_skipped_with_the_not_relative_reason() {
    let directory = tempfile::tempdir().expect("temp dir");
    let outside = tempfile::tempdir().expect("outside dir");
    let outside_file = outside.path().join("escaped.rs");

    let skipped = strip_project_prefix(directory.path(), &outside_file)
        .expect_err("a path outside the root must be reported, not stripped");

    assert_eq!(
        skipped.absolute_path, outside_file,
        "the skip must carry the path that could not be made relative"
    );
    assert_eq!(
        skipped.reason,
        DiscoveryError::NotRelative(outside_file.clone()).to_string(),
        "the skip reason must be the typed not-relative reason"
    );
    assert!(
        skipped
            .reason
            .contains("is not relative to the project root"),
        "the reason must explain why the file was left out: {}",
        skipped.reason
    );
}

/// A regular file discovered in one sync can be replaced by a symlink
/// before the next sync's walk (e.g. an editor or a malicious actor
/// swapping it out between syncs). `follow_links(false)` means the
/// walker must never dereference it: it either lists the symlink itself
/// (still classified without following it into whatever it points at)
/// or the walk simply doesn't traverse through it — either way, content
/// from outside the project root must never be sniffed/read as if it
/// were the original file.
#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_symlink_between_syncs_is_never_dereferenced() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, "lib.rs", b"pub fn a() {}\n");

    let (first, _skipped) = discover(root).expect("first discover");
    assert_eq!(
        first
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["lib.rs"]
    );

    // Outside the project root entirely, so a NUL byte here would prove
    // the walker actually dereferenced the link if it ever got sniffed.
    let outside = tempfile::tempdir().expect("outside dir");
    let secret = outside.path().join("secret.bin");
    fs::write(&secret, [0_u8, 1, 2]).expect("write outside file");

    std::fs::remove_file(root.join("lib.rs")).expect("remove the real file");
    std::os::unix::fs::symlink(&secret, root.join("lib.rs")).expect("replace with a symlink");

    let (second, _skipped) = discover(root).expect("second discover does not error or panic");
    // `follow_links(false)` makes walkdir/ignore report the symlink's own
    // (non-regular) file type via `lstat` rather than following it, so
    // `discover`'s `is_file()` filter drops it entirely: the replaced
    // path is safely skipped, not silently reindexed as if it still held
    // lib.rs's real content, and never dereferenced into the target
    // living outside `root`.
    assert_eq!(
        second,
        Vec::new(),
        "a path replaced by a symlink must be skipped, not reindexed or dereferenced"
    );
}

/// Scratch, temp, and editor-swap files must never be indexed — their
/// parse failures would otherwise pollute the project's evidence with
/// noise that has nothing to do with the source the project intends to
/// maintain. This covers both AI-generated scratch output (the
/// `.claude_output.txt` pattern that dominates real-world noise) and the
/// generic temp/backup patterns an editor or human leaves behind.
#[test]
fn scratch_and_temp_files_are_excluded_from_discovery() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, "src/lib.rs", b"pub fn lib() {}");
    write(root, "scratch.py", b"print('scratch')");
    write(root, "session.tmp", b"temp data");
    write(root, "notes.bak", b"backup");
    write(root, "buffer.swp", b"swap");
    write(root, "file~", b"emacs backup");
    write(root, "output.claude_output.txt", b"claude session output");

    let (files, _skipped) = discover(root).expect("discover");
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["src/lib.rs"],
        "only the real source file should be discovered, scratch/temp files must be excluded, got: {paths:?}"
    );
}

/// `sniff_kind` must report a missing path as `DiscoveryError::Read`, not
/// panic — a path that vanishes between the walk and the sniff (or is
/// unreadable) must not take down the whole sync.
#[test]
fn sniff_kind_reports_an_error_for_a_missing_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let error = sniff_kind(&directory.path().join("missing.rs"))
        .expect_err("a missing file must be a reported error, not a panic");
    assert!(matches!(error, DiscoveryError::Read { .. }));
}

/// Opening a directory succeeds on Unix but reading it fails with EISDIR —
/// the read-error arm of `sniff_kind`, the same failure a file that
/// becomes unreadable between walk and sniff would produce.
#[cfg(unix)]
#[test]
fn sniff_kind_reports_an_error_for_an_unreadable_path() {
    let directory = tempfile::tempdir().expect("temp dir");
    let dir = directory.path().join("dir");
    fs::create_dir(&dir).expect("create dir");

    let error = sniff_kind(&dir).expect_err("reading a directory must error");
    assert!(matches!(error, DiscoveryError::Read { .. }));
}

/// A root the walker cannot list yields one walk error and no entries.
/// Discovery must fail closed: returning `Ok` with an empty file set would
/// let the publish diff delete every stored row.
#[test]
fn a_missing_project_root_fails_discovery_with_the_walk_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let missing = directory.path().join("gone");

    let error = discover(&missing).expect_err("an unlistable root must fail discovery");

    assert!(
        matches!(error, DiscoveryError::Walk(_)),
        "the typed walk failure must survive, got {error:?}"
    );
    assert!(
        error.to_string().contains("failed to walk project files"),
        "the failure must name the walk: {error}"
    );
}

/// An unreadable directory deep in the tree is walked as an `Err` entry,
/// not as a file. It must name the failing directory (carried by the
/// walker's error), never the project root, and must not become a skip.
#[test]
fn a_walk_error_is_returned_with_the_failing_subdirectory_not_the_root() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    let failing = root.join("private-dir");
    let error = ignore::Error::WithPath {
        path: failing.clone(),
        err: Box::new(ignore::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ))),
    };
    let mut discovered = Vec::new();
    let mut skipped = Vec::new();

    let error = collect_walk_entry(root, Err(error), &mut discovered, &mut skipped)
        .expect_err("a walk error must fail discovery");

    match error {
        DiscoveryError::Walk(error) => assert_eq!(
            crate::watch::errors::ignore_error_path(&error),
            Some(failing.as_path()),
            "the failure must name the directory the walker could not read"
        ),
        other => panic!("expected DiscoveryError::Walk, got {other:?}"),
    }
    assert!(
        discovered.is_empty() && skipped.is_empty(),
        "a walk error is not a per-file skip: discovered={discovered:?} skipped={skipped:?}"
    );
}

/// A malformed `.gitignore` is attached to a walked entry as an error while
/// the walk continues with that rule dropped. Discovery must fail closed
/// instead of indexing a file set the user's ignore rules refute.
#[test]
fn a_malformed_gitignore_fails_discovery_instead_of_dropping_its_rules() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, ".gitignore", b"[z-a]\n");
    write(root, "kept.rs", b"fn kept() {}");

    let error = discover(root).expect_err("an unapplied ignore file must fail discovery");

    match error {
        DiscoveryError::IgnoreRules { kind, path } => {
            assert_eq!(kind, "glob", "the static category names the parse failure");
            let path = path.expect("the walker names the ignore file");
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".gitignore")
            );
        }
        other => panic!("expected DiscoveryError::IgnoreRules, got {other:?}"),
    }
}

/// The end-to-end consequence the fail-closed rule exists for: after a
/// successful publish, a root the walker cannot list must not produce a
/// revision that deletes the stored rows or shrinks the index.
#[test]
fn a_failed_walk_publishes_no_revision_and_deletes_no_stored_rows() {
    use crate::progress::NoopProgressSink;
    use crate::store::open_read_write;
    use crate::sync::PublishError;
    use crate::sync::publish::publish;
    use crate::sync::test_support::stored_paths;

    let base = tempfile::tempdir().expect("base dir");
    let root = base.path().join("root");
    fs::create_dir_all(&root).expect("create project root");
    write(&root, "a.rs", b"fn a() {}");
    let db_dir = tempfile::tempdir().expect("db dir");
    let mut connection = open_read_write(&db_dir.path().join("project.db")).expect("open database");
    let first = publish(&mut connection, &root, "1.0", &NoopProgressSink)
        .expect("initial publish succeeds");

    let moved = base.path().join("moved-away");
    fs::rename(&root, &moved).expect("move the project root away");

    let error = publish(&mut connection, &root, "1.0", &NoopProgressSink)
        .expect_err("an unlistable root must fail the publish");

    assert!(
        matches!(error, PublishError::Discovery(DiscoveryError::Walk(_))),
        "the publish must carry the typed discovery failure: {error:?}"
    );
    assert_eq!(
        stored_paths(&connection),
        ["a.rs"],
        "no stored row may be deleted by a partial walk"
    );
    let current: String = connection
        .query_row(
            "SELECT revision_id FROM revisions WHERE is_current = 1",
            [],
            |row| row.get(0),
        )
        .expect("read the current revision");
    assert_eq!(
        current, first.revision_id,
        "no revision may be published from a partial walk"
    );
}
