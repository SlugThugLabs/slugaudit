use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult};
use std::fs;

/// Makes `root` an active project: the activation directory is the switch.
fn create_activation(root: &Path) -> TestResult {
    fs::create_dir_all(root.join(PLANNING_DIR).join(ACTIVATION_DIR))
        .check("create activation dir")?;
    Ok(())
}

#[test]
fn resolve_project_returns_root_and_database_path_together() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    create_activation(directory.path())?;
    let (root, db) = resolve_project(directory.path()).check("active project")?;
    assert_eq!(
        root.as_path(),
        directory
            .path()
            .canonicalize()
            .check("tempdir canonicalizes")?
    );
    assert_eq!(db, database_path(&root));
    Ok(())
}

#[test]
fn resolve_project_fails_when_no_project_is_active() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let error = resolve_project(directory.path()).check_err("no activation directory")?;
    assert!(matches!(error, ActivationError::NotActive));
    Ok(())
}

#[test]
fn finds_an_active_project_at_its_own_root() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    create_activation(directory.path())?;
    let root = find_project_root(directory.path()).check("active project")?;
    assert_eq!(
        root.as_path(),
        directory
            .path()
            .canonicalize()
            .check("tempdir canonicalizes")?
    );
    Ok(())
}

#[test]
fn finds_an_active_project_from_a_nested_file() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    create_activation(directory.path())?;
    let nested = directory.path().join("src").join("nested");
    fs::create_dir_all(&nested).check("create nested dir")?;
    let file = nested.join("main.rs");
    fs::write(&file, b"fn main() {}").check("write fixture file")?;

    let root = find_project_root(&file).check("active project")?;
    assert_eq!(
        root.as_path(),
        directory
            .path()
            .canonicalize()
            .check("tempdir canonicalizes")?
    );
    Ok(())
}

#[test]
fn rejects_a_project_with_no_activation_directory() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
    Ok(())
}

#[test]
fn a_start_path_that_does_not_exist_is_an_unresolvable_root_not_an_inactive_project() -> TestResult
{
    let directory = tempfile::tempdir().check("temp dir")?;
    let missing = directory.path().join("no-such-dir");

    let result = find_project_root(&missing);

    assert!(
        matches!(
            result,
            Err(ActivationError::Root(RootError::Unresolvable(_)))
        ),
        "a missing path must say it cannot be resolved, got {result:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlinked_activation_directory_does_not_count_as_on() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).check("create link target")?;
    fs::create_dir_all(directory.path().join(PLANNING_DIR)).check("create planning dir")?;
    std::os::unix::fs::symlink(
        &real_target,
        directory.path().join(PLANNING_DIR).join(ACTIVATION_DIR),
    )
    .check("create symlink")?;

    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_symlinked_planning_directory_is_refused_when_looking_for_a_project() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(real_target.join(ACTIVATION_DIR)).check("create link target")?;
    std::os::unix::fs::symlink(&real_target, directory.path().join(PLANNING_DIR))
        .check("create symlink")?;

    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::SymlinkedActivationPath)
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn enable_refuses_a_symlinked_activation_directory() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).check("create link target")?;
    fs::create_dir_all(directory.path().join(PLANNING_DIR)).check("create planning dir")?;
    std::os::unix::fs::symlink(
        &real_target,
        directory.path().join(PLANNING_DIR).join(ACTIVATION_DIR),
    )
    .check("create symlink")?;
    let root = ProjectRoot::resolve(directory.path()).check("resolve root")?;

    assert!(matches!(
        enable(&root),
        Err(ActivationError::SymlinkedActivationPath)
    ));
    Ok(())
}

#[test]
fn enable_creates_the_directory_and_that_alone_makes_the_project_active() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("resolve root")?;
    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));

    let created = enable(&root).check("enable succeeds")?;

    assert!(created.is_dir());
    assert_eq!(
        find_project_root(directory.path())
            .check("now active")?
            .as_path(),
        root.as_path()
    );
    assert!(
        !created.join("project.db").exists(),
        "enable builds no database"
    );
    Ok(())
}

#[test]
fn enable_is_idempotent() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let root = ProjectRoot::resolve(directory.path()).check("resolve root")?;
    enable(&root).check("first enable")?;
    assert!(
        enable(&root).is_ok(),
        "enabling an already-active project must not error"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn enable_refuses_to_create_through_a_symlinked_planning_dir() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let real_target = directory.path().join("elsewhere");
    fs::create_dir_all(&real_target).check("create link target")?;
    std::os::unix::fs::symlink(&real_target, directory.path().join(PLANNING_DIR))
        .check("create symlink")?;
    let root = ProjectRoot::resolve(directory.path()).check("resolve root")?;

    assert!(matches!(
        enable(&root),
        Err(ActivationError::SymlinkedActivationPath)
    ));
    Ok(())
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
fn an_activation_directory_toggled_concurrently_never_panics_or_returns_a_partial_state()
-> TestResult {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let directory = tempfile::tempdir().check("temp dir")?;
    let root = directory.path().to_path_buf();
    create_activation(&root)?;
    let nested = root.join("src").join("nested");
    fs::create_dir_all(&nested).check("create nested dir")?;
    let canonical_root = root.canonicalize().check("canonicalize root")?;

    let activation_dir = root.join(PLANNING_DIR).join(ACTIVATION_DIR);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_toggler = Arc::clone(&stop);
    let toggler = std::thread::spawn(move || -> TestResult {
        while !stop_toggler.load(Ordering::Relaxed) {
            fs::remove_dir_all(&activation_dir).check("toggler removes the directory")?;
            fs::create_dir_all(&activation_dir).check("toggler recreates the directory")?;
        }
        Ok(())
    });

    for _ in 0..2_000 {
        match find_project_root(&nested) {
            Ok(found) => assert_eq!(found.as_path(), canonical_root),
            Err(ActivationError::NotActive) => {}
            Err(other) => {
                return Err(format!(
                    "a benign concurrent marker toggle must never produce {other:?}"
                ))
                .check("unexpected outcome");
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    crate::util::join_thread(toggler).check("toggler thread joins")??;
    Ok(())
}

fn overlong_path() -> PathBuf {
    Path::new("/tmp").join("a".repeat(300))
}

#[test]
fn an_uninspectable_symlink_guard_path_fails_the_activation_check_closed() {
    let result = planning_checked_activation_dir(&overlong_path());

    assert!(matches!(result, Err(ActivationError::Io(_))), "{result:?}");
}

#[test]
fn an_unstattable_activation_path_is_an_error_not_absent() -> TestResult {
    let missing = tempfile::tempdir().check("temp dir")?.path().join("gone");

    assert!(!is_active_at(&missing).check("a missing path is absent")?);
    assert!(is_active_at(&overlong_path()).is_err());
    Ok(())
}

#[test]
fn a_directory_without_project_db_is_an_active_project() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let activation = directory.path().join(PLANNING_DIR).join(ACTIVATION_DIR);
    fs::create_dir_all(&activation).check("create activation dir")?;
    fs::write(activation.join("config.json"), b"{}").check("write config")?;

    assert!(find_project_root(directory.path()).is_ok());
    assert!(resolve_project(directory.path()).is_ok());
    Ok(())
}

#[test]
fn an_activation_path_that_is_a_regular_file_is_not_active() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let planning = directory.path().join(PLANNING_DIR);
    fs::create_dir_all(&planning).check("create planning dir")?;
    fs::write(planning.join(ACTIVATION_DIR), b"x").check("write file")?;

    assert!(matches!(
        find_project_root(directory.path()),
        Err(ActivationError::NotActive)
    ));
    Ok(())
}

#[path = "activation_disable_tests.rs"]
mod disable_tests;

#[path = "activation_gitignore_tests.rs"]
mod gitignore_tests;
