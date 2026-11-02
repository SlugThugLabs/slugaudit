use super::activation::activation_dir;
use super::root::ProjectRoot;
use std::path::PathBuf;

pub(super) const DATABASE_FILENAME: &str = "project.db";

/// Returns the SlugAudit-owned runtime database inside a user project:
/// `<project-root>/.planning/slugaudit/project.db`.
///
/// The surrounding `.planning` directory belongs to the user project;
/// SlugAudit owns only the `slugaudit` child directory. This function never
/// accepts a caller-supplied database path.
#[must_use]
pub fn database_path(root: &ProjectRoot) -> PathBuf {
    activation_dir(root).join(DATABASE_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, TestResult};

    #[test]
    fn database_path_stays_inside_the_activation_directory() -> TestResult {
        let directory = tempfile::tempdir().check("temp dir")?;
        std::fs::create_dir_all(directory.path().join(".planning").join("slugaudit"))
            .check("create activation dir")?;
        let root = ProjectRoot::resolve(directory.path()).check("valid root")?;

        let path = database_path(&root);
        assert_eq!(
            path,
            directory
                .path()
                .canonicalize()
                .check("tempdir canonicalizes")?
                .join(".planning")
                .join("slugaudit")
                .join("project.db")
        );
        Ok(())
    }
}
