//! Shared fixture helper for `tools` tests. `activated_project` creates a
//! temp project with the `.planning/slugaudit/` activation directory and one
//! source file; it is used by finding_tests.rs,
//! context_tests.rs, and structure_limit_tests.rs.
use std::fs;

/// Creates an enabled (activation directory present) temp project containing
/// `relative` with `content`.
pub(crate) fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"));
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}

/// Keeps a temporary global config in place for the calling test thread; the
/// real `~/.slugthug` is never read in tests. Dropping it removes the config.
pub(crate) struct GlobalConfigGuard {
    _dir: tempfile::TempDir,
}

/// Makes `contents` the calling thread's global `config.json`.
pub(crate) fn use_global_config(contents: &str) -> GlobalConfigGuard {
    let dir = tempfile::tempdir().expect("global config dir");
    let path = dir.path().join("config.json");
    fs::write(&path, contents).expect("write global config");
    crate::project::set_global_config_path_for_test(Some(path));
    GlobalConfigGuard { _dir: dir }
}

impl Drop for GlobalConfigGuard {
    fn drop(&mut self) {
        crate::project::set_global_config_path_for_test(None);
    }
}

/// This process's session findings database for `project`
/// (`findings-<id>.db`). The project must already have had a tool call, which
/// starts the session.
pub(crate) fn own_findings_path(project: &tempfile::TempDir) -> std::path::PathBuf {
    let root = crate::project::ProjectRoot::resolve(project.path()).expect("project root");
    crate::project::session_findings_path(&crate::project::activation_dir(&root))
        .expect("a tool call started this process's session for the project")
}
