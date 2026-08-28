use super::activation::activation_dir;
use super::root::ProjectRoot;
use std::path::PathBuf;

const DATABASE_FILENAME: &str = "project.db";

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

    #[test]
    fn database_path_stays_inside_the_activation_directory() {
        let directory = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(directory.path().join(".planning").join("slugaudit"))
            .expect("create activation dir");
        let root = ProjectRoot::resolve(directory.path()).expect("valid root");

        let path = database_path(&root);
        assert_eq!(
            path,
            directory
                .path()
                .canonicalize()
                .expect("tempdir canonicalizes")
                .join(".planning")
                .join("slugaudit")
                .join("project.db")
        );
    }
}
