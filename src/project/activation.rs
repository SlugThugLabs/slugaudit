use super::root::{ProjectRoot, RootError};
use std::path::{Path, PathBuf};
use thiserror::Error;

const PLANNING_DIR: &str = ".planning";
const ACTIVATION_DIR: &str = "slugaudit";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActivationError {
    #[error(transparent)]
    Root(#[from] RootError),
    #[error("SlugAudit is not enabled for this project or any parent directory")]
    NotActive,
    #[error("refusing a symlinked SlugAudit activation path")]
    SymlinkedActivationPath,
}

/// The `.planning/slugaudit` directory under a resolved project root.
#[must_use]
pub fn activation_dir(root: &ProjectRoot) -> PathBuf {
    root.as_path().join(PLANNING_DIR).join(ACTIVATION_DIR)
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Returns the activation directory under `canonical_root`, refusing a
/// symlinked `.planning` or `.planning/slugaudit` component.
fn validated_activation_dir(canonical_root: &Path) -> Result<PathBuf, ActivationError> {
    let planning = canonical_root.join(PLANNING_DIR);
    let activation = planning.join(ACTIVATION_DIR);
    if is_symlink(&planning) || is_symlink(&activation) {
        return Err(ActivationError::SymlinkedActivationPath);
    }
    Ok(activation)
}

/// Finds the nearest ancestor of `start` (inclusive) whose activation
/// directory exists. `start` may be a file, in which case its parent
/// directory is the search origin.
///
/// # Errors
///
/// Returns an error if `start` cannot be resolved, if a `.planning` or
/// `.planning/slugaudit` component along the way is a symlink, or if no
/// ancestor has an activation directory at all.
pub fn find_project_root(start: &Path) -> Result<ProjectRoot, ActivationError> {
    let canonical = start
        .canonicalize()
        .map_err(|error| ActivationError::Root(RootError::Unresolvable(error.to_string())))?;
    let search_origin = if canonical.is_file() {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .ok_or(ActivationError::NotActive)?
    } else {
        canonical
    };

    for ancestor in search_origin.ancestors() {
        let activation = validated_activation_dir(ancestor)?;
        if activation.is_dir() {
            return Ok(ProjectRoot::resolve(ancestor)?);
        }
    }
    Err(ActivationError::NotActive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_activation(root: &Path) {
        fs::create_dir_all(root.join(PLANNING_DIR).join(ACTIVATION_DIR))
            .expect("create activation dir");
    }

    #[test]
    fn finds_an_active_project_at_its_own_root() {
        let directory = tempfile::tempdir().expect("temp dir");
        create_activation(directory.path());
        let root = find_project_root(directory.path()).expect("active project");
        assert_eq!(root.as_path(), directory.path().canonicalize().unwrap());
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
        assert_eq!(root.as_path(), directory.path().canonicalize().unwrap());
    }

    #[test]
    fn rejects_a_project_with_no_activation_marker() {
        let directory = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            find_project_root(directory.path()),
            Err(ActivationError::NotActive)
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

        assert_eq!(
            find_project_root(directory.path()),
            Err(ActivationError::SymlinkedActivationPath)
        );
    }

    /// This crate never creates or removes the activation marker itself
    /// (see README.md's "Activation ownership" section) — the realistic race
    /// is a host application toggling `.planning/slugaudit` concurrently with
    /// a lookup, simulating disabling/enabling the project mid-call.
    /// `find_project_root` is a single synchronous ancestor walk with no
    /// hook to pause it deterministically mid-loop, so this drives the race
    /// with a real second thread hammering create/remove on the marker
    /// directory while many real lookups run concurrently, asserting the
    /// only two acceptable outcomes (a clean success matching the real root,
    /// or a clean `NotActive`) and never a panic or any other error variant.
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
                let _ = fs::remove_dir_all(&activation_dir);
                let _ = fs::create_dir_all(&activation_dir);
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
}
