use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RootError {
    #[error("failed to resolve project root: {0}")]
    Unresolvable(String),
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
        let canonical = path
            .canonicalize()
            .map_err(|error| RootError::Unresolvable(error.to_string()))?;
        if !canonical.is_dir() {
            return Err(RootError::NotADirectory);
        }
        Ok(Self(canonical))
    }

    #[must_use]
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
        assert_eq!(root.as_path(), directory.path().canonicalize().unwrap());
    }

    #[test]
    fn rejects_a_file_as_root() {
        let directory = tempfile::tempdir().expect("temp dir");
        let file_path = directory.path().join("not-a-directory");
        std::fs::write(&file_path, b"content").expect("write fixture file");
        assert_eq!(
            ProjectRoot::resolve(&file_path),
            Err(RootError::NotADirectory)
        );
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
}
