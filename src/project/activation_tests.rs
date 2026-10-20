use super::*;
use std::fs;

fn create_activation(root: &Path) {
    fs::create_dir_all(root.join(PLANNING_DIR).join(ACTIVATION_DIR))
        .expect("create activation dir");
}

#[cfg(unix)]
#[test]
fn enabling_a_project_creates_an_owner_only_activation_directory() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");

    let activation = enable(&root).expect("enable");

    let mode = fs::metadata(&activation)
        .expect("stat activation dir")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "the index directory holds source text");
}

/// `create_dir_all` honours the umask, so a project enabled on a machine with
/// a permissive one — or enabled by an older build — can already exist wider
/// than `0700`. Re-enabling must not leave it that way.
#[cfg(unix)]
#[test]
fn enabling_a_project_tightens_a_pre_existing_wider_activation_directory() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temp dir");
    let activation = directory.path().join(PLANNING_DIR).join(ACTIVATION_DIR);
    fs::create_dir_all(&activation).expect("pre-create wider");
    fs::set_permissions(&activation, fs::Permissions::from_mode(0o755)).expect("widen");

    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let enabled = enable(&root).expect("enable");

    let mode = fs::metadata(&enabled).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "a wider existing directory is tightened");
}

#[test]
fn resolve_project_returns_root_and_database_path_together() {
    let directory = tempfile::tempdir().expect("temp dir");
    create_activation(directory.path());
    let (root, db) = resolve_project(directory.path()).expect("active project");
    assert_eq!(
        root.as_path(),
        directory
            .path()
            .canonicalize()
            .expect("tempdir canonicalizes")
    );
    assert_eq!(db, database_path(&root));
}

#[test]
fn resolve_project_fails_when_no_project_is_active() {
    let directory = tempfile::tempdir().expect("temp dir");
    let error = resolve_project(directory.path()).expect_err("no activation marker");
    assert!(matches!(error, ActivationError::NotActive));
}

#[test]
fn finds_an_active_project_at_its_own_root() {
    let directory = tempfile::tempdir().expect("temp dir");
    create_activation(directory.path());
    let root = find_project_root(directory.path()).expect("active project");
    assert_eq!(
        root.as_path(),
        directory
            .path()
            .canonicalize()
            .expect("tempdir canonicalizes")
    );
}

#[test]
fn finds_an_active_project_from_a_nested_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    create_activation(directory.path());
    let nested = directory.path().join("src").join("nested");
    fs::create_dir_all(&nested).expect("create nested dir");
    let file = nested.join("main.rs");
    fs::write(&file, b"fn main() {}").expect("write fixture file");

    let root = find_project_root(&file).expect("active project");
    assert_eq!(
        root.as_path(),
        directory
            .path()
            .canonicalize()
            .expect("tempdir canonicalizes")
    );
}

#[test]
fn rejects_a_project_with_no_activation_marker() {
    let directory = tempfile::tempdir().expect("temp dir");
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[test]
fn a_start_path_that_does_not_exist_is_an_unresolvable_root_not_an_inactive_project() {
    let directory = tempfile::tempdir().expect("temp dir");
    let missing = directory.path().join("no-such-dir");

    let result = find_project_root(&missing);

    assert!(
        matches!(
            result,
            Err(ActivationError::Root(RootError::Unresolvable(_)))
        ),
        "a missing path must say it cannot be resolved, got {result:?}"
    );
}

#[cfg(unix)]
#[test]
fn rejects_a_symlinked_activation_directory() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).expect("create link target");
    fs::create_dir_all(directory.path().join(PLANNING_DIR)).expect("create planning dir");
    std::os::unix::fs::symlink(
        &real_target,
        directory.path().join(PLANNING_DIR).join(ACTIVATION_DIR),
    )
    .expect("create symlink");

    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::SymlinkedActivationPath)
    ));
}

#[test]
fn enable_creates_the_marker_and_find_project_root_then_sees_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    let created = enable(&root).expect("enable succeeds");
    assert!(created.is_dir());
    assert_eq!(
        find_project_root(directory.path())
            .expect("now active")
            .as_path(),
        root.as_path()
    );
}

#[test]
fn enable_is_idempotent() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    enable(&root).expect("first enable");
    assert!(
        enable(&root).is_ok(),
        "enabling an already-active project must not error"
    );
}

#[test]
fn disable_removes_the_marker_and_find_project_root_stops_seeing_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    enable(&root).expect("enable");

    let removed = disable(&root).expect("disable succeeds");
    assert!(
        removed,
        "an active project's disable must report it actually removed something"
    );
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
}

#[test]
fn disabling_an_already_inactive_project_is_a_no_op_not_an_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");
    let removed = disable(&root).expect("disable succeeds even when never enabled");
    assert!(!removed);
}

#[cfg(unix)]
#[test]
fn enable_refuses_to_create_through_a_symlinked_planning_dir() {
    let directory = tempfile::tempdir().expect("temp dir");
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).expect("create link target");
    std::os::unix::fs::symlink(&real_target, directory.path().join(PLANNING_DIR))
        .expect("create symlink");
    let root = ProjectRoot::resolve(directory.path()).expect("resolve root");

    assert!(matches!(
        enable(&root),
        Err(ActivationError::SymlinkedActivationPath)
    ));
}

/// `find_project_root` is a read-only lookup that can race a concurrent
/// toggle of `.planning/slugaudit` — whether that's the `project_control`
/// MCP tool's `enable`/`disable` or a host application doing the same
/// thing directly, the marker can be created or removed mid-lookup. `find_project_root` is a single synchronous ancestor
/// walk with no hook to pause it deterministically mid-loop, so this
/// drives the race with a real second thread hammering create/remove
/// on the marker directory while many real lookups run concurrently,
/// asserting the only two acceptable outcomes (a clean success
/// matching the real root, or a clean `NotActive`) and never a panic
/// or any other error variant.
#[cfg(unix)]
#[test]
fn a_marker_toggled_concurrently_never_panics_or_returns_a_partial_state() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path().to_path_buf();
    create_activation(&root);
    let nested = root.join("src").join("nested");
    fs::create_dir_all(&nested).expect("create nested dir");
    let canonical_root = root.canonicalize().expect("canonicalize root");

    let activation_dir = root.join(PLANNING_DIR).join(ACTIVATION_DIR);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_toggler = Arc::clone(&stop);
    let toggler = std::thread::spawn(move || {
        while !stop_toggler.load(Ordering::Relaxed) {
            fs::remove_dir_all(&activation_dir).expect("toggler removes the marker");
            fs::create_dir_all(&activation_dir).expect("toggler recreates the marker");
        }
    });

    for _ in 0..2_000 {
        match find_project_root(&nested) {
            Ok(found) => assert_eq!(found.as_path(), canonical_root),
            Err(ActivationError::NotActive) => {}
            Err(other) => {
                panic!("a benign concurrent marker toggle must never produce {other:?}")
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    toggler.join().expect("toggler thread joins");
}

/// Resolves `directory` and writes `contents` to its root `.gitignore`,
/// returning both the root and the file's path.
fn gitignore_fixture(directory: &tempfile::TempDir, contents: &[u8]) -> (ProjectRoot, PathBuf) {
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let path = root.as_path().join(GITIGNORE_FILE);
    fs::write(&path, contents).expect("write .gitignore");
    (root, path)
}

#[test]
fn a_project_without_a_gitignore_gains_the_rule_in_a_new_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let path = root.as_path().join(GITIGNORE_FILE);

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        b".planning/slugaudit/\n".to_vec(),
        "the created file holds exactly the one rule"
    );
}

#[test]
fn appending_keeps_every_existing_byte_of_the_gitignore() {
    let directory = tempfile::tempdir().expect("temp dir");
    let original = b"/target\n*.log\n";
    let (root, path) = gitignore_fixture(&directory, original);

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    let written = fs::read(&path).expect("read .gitignore");
    assert!(
        written.starts_with(original),
        "the user's bytes must come through unchanged: {written:?}"
    );
    assert_eq!(&written[original.len()..], b".planning/slugaudit/\n");
}

#[test]
fn an_equivalent_rule_is_recognised_and_not_duplicated() {
    for rule in [
        ".planning/slugaudit/",
        ".planning/slugaudit",
        "/.planning/slugaudit/",
        "/.planning/slugaudit",
    ] {
        let directory = tempfile::tempdir().expect("temp dir");
        let contents = format!("{rule}\n");
        let (root, path) = gitignore_fixture(&directory, contents.as_bytes());

        let outcome = ensure_gitignore(&root);

        assert_eq!(
            outcome,
            GitignoreOutcome::AlreadyIgnored(path.clone()),
            "rule {rule:?} already ignores the directory"
        );
        assert_eq!(
            fs::read(&path).expect("read .gitignore"),
            contents.as_bytes(),
            "an already-ignored project's file stays byte-identical"
        );
    }
}

#[test]
fn a_commented_or_negated_line_does_not_make_the_directory_ignored() {
    let directory = tempfile::tempdir().expect("temp dir");
    let contents = b"# .planning/slugaudit/\n!.planning/slugaudit/\n.planning/slugaudit-extra/\n";
    let (root, path) = gitignore_fixture(&directory, contents);

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert!(
        fs::read(&path)
            .expect("read .gitignore")
            .ends_with(b"\n.planning/slugaudit/\n"),
        "the real rule is appended after the decoys"
    );
}

#[test]
fn a_last_line_without_a_newline_is_not_merged_with_the_rule() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (root, path) = gitignore_fixture(&directory, b"/target");

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        b"/target\n.planning/slugaudit/\n".to_vec(),
        "the user's last line keeps its own line"
    );
}

#[test]
fn an_empty_gitignore_file_gains_only_the_rule() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (root, path) = gitignore_fixture(&directory, b"");

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        b".planning/slugaudit/\n".to_vec(),
        "no blank line is introduced ahead of the rule"
    );
}

#[test]
fn a_crlf_gitignore_keeps_its_line_ending_convention() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (root, path) = gitignore_fixture(&directory, b"/target\r\n");

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        b"/target\r\n.planning/slugaudit/\r\n".to_vec(),
        "the appended rule uses CRLF, not a mixed ending"
    );
}

#[test]
fn a_crlf_gitignore_without_a_final_newline_gets_a_crlf_separator() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (root, path) = gitignore_fixture(&directory, b"/a\r\n/b");

    let outcome = ensure_gitignore(&root);

    assert_eq!(outcome, GitignoreOutcome::Added(path.clone()));
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        b"/a\r\n/b\r\n.planning/slugaudit/\r\n".to_vec(),
        "the separator matches the file's convention too"
    );
}

#[test]
fn an_unreadable_gitignore_is_reported_rather_than_fatal() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let path = root.as_path().join(GITIGNORE_FILE);
    fs::create_dir(&path).expect("a directory where the ignore file belongs");

    let outcome = ensure_gitignore(&root);

    match outcome {
        GitignoreOutcome::NotWritten {
            path: reported,
            reason,
        } => {
            assert_eq!(reported, path, "the unwritable path is named");
            assert!(!reason.is_empty(), "the reason is reported");
        }
        other => panic!("a file that cannot be read must report NotWritten, got {other:?}"),
    }
}

/// `NotWritten` promises the file was left untouched. The write is atomic
/// (temp file in the same directory, then rename), so even a failure after
/// the read cannot have created the path or appended a partial rule — and
/// no temp file may survive the failure.
#[test]
fn a_failed_gitignore_write_leaves_the_original_untouched() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = ProjectRoot::resolve(directory.path()).expect("valid root");
    let path = root.as_path().join(GITIGNORE_FILE);
    fs::create_dir(&path).expect("a directory where the ignore file belongs");
    let marker = path.join("keep");
    fs::write(&marker, b"x").expect("write a marker inside the directory");

    let outcome = ensure_gitignore(&root);

    assert!(matches!(outcome, GitignoreOutcome::NotWritten { .. }));
    assert!(
        path.is_dir(),
        "the original path is still the same directory"
    );
    assert_eq!(
        fs::read(&marker).expect("read the marker"),
        b"x",
        "nothing inside the original path was touched"
    );
    let leftovers = fs::read_dir(root.as_path())
        .expect("read the project root")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".gitignore.slugaudit.")
        })
        .count();
    assert_eq!(leftovers, 0, "no temp file survives a failed write");
}

#[test]
fn an_oversized_gitignore_is_reported_and_left_byte_identical() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cap = usize::try_from(MAX_FOREIGN_TEXT_BYTES).expect("cap fits usize");
    let contents = vec![b'#'; cap + 1];
    let (root, path) = gitignore_fixture(&directory, &contents);

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
        other => panic!("an over-cap .gitignore must be reported, not read, got {other:?}"),
    }
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        contents,
        "an over-cap file is never truncated or appended to"
    );
}

#[test]
fn a_gitignore_exactly_at_the_cap_is_still_inspected() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cap = usize::try_from(MAX_FOREIGN_TEXT_BYTES).expect("cap fits usize");
    let rule = b".planning/slugaudit/\n";
    let mut contents = vec![b'\n'; cap - rule.len()];
    contents.extend_from_slice(rule);
    assert_eq!(contents.len(), cap, "the fixture sits exactly on the cap");
    let (root, path) = gitignore_fixture(&directory, &contents);

    let outcome = ensure_gitignore(&root);

    assert_eq!(
        outcome,
        GitignoreOutcome::AlreadyIgnored(path.clone()),
        "a file exactly at the cap is inspected, not rejected"
    );
    assert_eq!(
        fs::read(&path).expect("read .gitignore"),
        contents,
        "inspecting the file leaves it byte-identical"
    );
}

fn overlong_path() -> PathBuf {
    Path::new("/tmp").join("a".repeat(300))
}

#[test]
fn an_uninspectable_symlink_guard_path_fails_the_activation_check_closed() {
    let result = validated_activation_dir(&overlong_path());

    assert!(matches!(result, Err(ActivationError::Io(_))), "{result:?}");
}

#[test]
fn an_unstattable_activation_directory_is_an_error_not_absent() {
    let missing = tempfile::tempdir().expect("temp dir").path().join("gone");

    assert!(!is_existing_dir(&missing).expect("a missing path is absent"));
    assert!(is_existing_dir(&overlong_path()).is_err());
}
