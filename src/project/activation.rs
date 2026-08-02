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
}
