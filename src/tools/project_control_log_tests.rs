//! Log-record tests for the `project_control` enable path: the corrupt
//! database discard WARN names the real database path (P4 real-path policy).

use super::*;
use std::fs;

/// The enable-path corrupt-database discard WARN names the real database
/// path and the raw source error (P4 real-path policy), not a redacted id.
#[test]
fn on_action_corrupt_database_discard_warn_logs_the_real_path() {
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("lib.rs"), "pub fn a() {}\n").expect("source file");
    let path = project.path().to_string_lossy().to_string();
    let activation = project.path().join(".planning").join("slugaudit");
    let database = activation.join("project.db");
    fs::create_dir_all(&activation).expect("db dir");
    fs::write(&database, vec![0xA5_u8; 4096]).expect("write a corrupt database");
    // Owner-only so the permission guard passes and the failure is the
    // corruption itself, not the file mode.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))
            .expect("restrict the corrupt database to owner-only");
    }
    let expected_path = database.display().to_string();

    let (result, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for enable_project_discard");
        project_control(
            &Parameters(ProjectControlRequest {
                action: ProjectControlAction::On,
                path: Some(path.clone()),
                profile: None,
            }),
            &crate::progress::NoopProgressSink,
            None,
        )
    });

    let response = result.expect("the corrupt database must be discarded and rebuilt");
    assert_eq!(response.0.status, "enabled");
    let record = logs
        .lines()
        .find(|line| line.contains("operation=\"enable_project\""))
        .unwrap_or_else(|| panic!("the discard record must exist:\n{logs}"));
    assert!(
        record.contains(&format!(" path={expected_path} ")),
        "the discard WARN must carry the real database path:\n{logs}"
    );
    assert!(
        record.contains("database is corrupt or outdated; discarding and re-importing"),
        "the record must be the discard WARN:\n{logs}"
    );
    assert!(
        !record.contains("database_id="),
        "the redacted database id is gone:\n{logs}"
    );
}
