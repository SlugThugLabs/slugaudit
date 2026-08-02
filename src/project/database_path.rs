use super::activation::activation_dir;
use super::root::ProjectRoot;
use std::path::PathBuf;

const DATABASE_FILENAME: &str = "project.db";

/// The one location a project's SQLite database is ever allowed to live:
/// inside its own activation directory. Never accepts a caller-supplied
/// path — there is no argument to override it with.
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
                .unwrap()
                .join(".planning")
                .join("slugaudit")
                .join("project.db")
        );
    }
}
