//! Log-content test for the health read-only open failure.

/// When the read-only open is rejected after the path check succeeds, the
/// health WARN names the real database path (`path=`) and the source error,
/// not a redacted `database_id`.
#[test]
fn health_open_warn_logs_the_real_path_and_source_error() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("dir");
    let path = directory.path().join("health_open_real_path.db");
    std::fs::write(&path, b"x").expect("write file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("widen the database mode");

    let (_, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for health_open_real_path");
        super::read_database(&path);
    });

    assert!(
        logs.contains("capture liveness control for health_open_real_path"),
        "the capture must be live: {logs}"
    );
    let open = logs
        .lines()
        .find(|line| line.contains("failed to open the project database read-only"))
        .unwrap_or_else(|| panic!("open record missing:\n{logs}"));
    assert!(
        open.contains(&format!(" path={} ", path.display())),
        "open WARN must carry the real path:\n{open}"
    );
    assert!(
        open.contains(" error=") && open.contains("broader than owner-only 600"),
        "source error missing:\n{open}"
    );
    assert!(
        !logs.contains("database_id="),
        "the redacted database id is gone:\n{logs}"
    );
}
