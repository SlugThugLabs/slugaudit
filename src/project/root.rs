use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
/// Why a path could not be resolved to an enabled project root.
pub enum RootError {
    #[error("failed to resolve project root: {0}")]
    Unresolvable(#[source] std::io::Error),
    #[error("project root is not a directory")]
    NotADirectory,
}

/// A canonical, on-disk project root directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot(PathBuf);

impl ProjectRoot {
    /// # Errors
    ///
    /// Returns an error if `path` cannot be resolved to a canonical path,
    /// or if it resolves to something other than a directory.
    pub fn resolve(path: &Path) -> Result<Self, RootError> {
        let canonical = path.canonicalize().map_err(RootError::Unresolvable)?;
        if !std::fs::metadata(&canonical)
            .map_err(RootError::Unresolvable)?
            .is_dir()
        {
            return Err(RootError::NotADirectory);
        }
        Ok(Self(canonical))
    }

    #[must_use]
    /// The resolved, canonical project root.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_an_existing_directory() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = ProjectRoot::resolve(directory.path()).expect("valid root");
        assert_eq!(
            root.as_path(),
            directory
                .path()
                .canonicalize()
                .expect("tempdir canonicalizes")
        );
    }

    #[test]
    fn rejects_a_file_as_root() {
        let directory = tempfile::tempdir().expect("temp dir");
        let file_path = directory.path().join("not-a-directory");
        std::fs::write(&file_path, b"content").expect("write fixture file");
        assert!(matches!(
            ProjectRoot::resolve(&file_path),
            Err(RootError::NotADirectory)
        ));
    }

    #[test]
    fn rejects_a_path_that_does_not_exist() {
        let directory = tempfile::tempdir().expect("temp dir");
        let missing = directory.path().join("does-not-exist");
        assert!(matches!(
            ProjectRoot::resolve(&missing),
            Err(RootError::Unresolvable(_))
        ));
    }

    #[test]
    fn an_unresolvable_root_keeps_the_io_error_kind() {
        let directory = tempfile::tempdir().expect("temp dir");
        let missing = directory.path().join("does-not-exist");

        let Err(RootError::Unresolvable(source)) = ProjectRoot::resolve(&missing) else {
            panic!("a missing path must be unresolvable");
        };

        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    }
}
