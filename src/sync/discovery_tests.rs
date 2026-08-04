use super::*;
use std::fs;

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

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
#[cfg(unix)]
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
