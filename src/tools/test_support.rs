//! Shared fixture helper for `tools` tests. `activated_project` creates a
//! temp project with the `.planning/slugaudit/` activation marker and one
//! source file; it was previously copied in finding_tests.rs,
//! context_tests.rs, and structure_limit_tests.rs.
use std::fs;

/// Creates an enabled (activation-marker present) temp project containing
/// `relative` with `content`.
pub(crate) fn activated_project(relative: &str, content: &[u8]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    fs::write(project.path().join(relative), content).expect("write fixture file");
    project
}
