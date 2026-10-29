//! `project_control` against a symlinked `project.db` (never followed) and the
//! watcher's lifetime across a refused `off`.

use super::*;
use std::fs;

const VICTIM_BYTES: &[u8] = b"not a SlugAudit database\n";

fn request(path: &str, action: ProjectControlAction) -> Parameters<ProjectControlRequest> {
    Parameters(ProjectControlRequest {
        path: Some(path.to_owned()),
        action,
        profile: None,
    })
}

/// A project whose `.planning/slugaudit/project.db` is a symlink to a regular
/// file outside the activation directory. Returns the project, its path, the
/// link and the target.
#[cfg(unix)]
fn project_with_symlinked_database() -> (
    tempfile::TempDir,
    String,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join("src")).expect("create src dir");
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").expect("write source");
    let activation = project.path().join(".planning").join("slugaudit");
    fs::create_dir_all(&activation).expect("create activation dir");
    let target = project.path().join("victim.db");
    fs::write(&target, VICTIM_BYTES).expect("write link target");
    let link = activation.join("project.db");
    std::os::unix::fs::symlink(&target, &link).expect("symlink project.db");
    let path = project.path().to_string_lossy().into_owned();
    (project, path, link, target)
}

#[cfg(unix)]
#[test]
fn a_symlinked_project_db_reads_as_inactive() {
    let (project, _path, _link, _target) = project_with_symlinked_database();

    let result = crate::project::find_project_root(project.path());

    assert!(
        matches!(result, Err(crate::project::ActivationError::NotActive)),
        "a symlinked project.db is not an activation marker: {result:?}"
    );
}

#[cfg(unix)]
#[test]
fn on_refuses_a_symlinked_project_db_and_leaves_the_link_and_target_alone() {
    let (_project, path, link, target) = project_with_symlinked_database();

    let error = project_control(
        &request(&path, ProjectControlAction::On),
        &crate::progress::NoopProgressSink,
        None,
    )
    .err()
    .expect("on must refuse a symlinked project.db");

    assert_eq!(
        error.message,
        "database: refusing to open a database path that is a symlink"
    );
    assert!(
        fs::symlink_metadata(&link)
            .expect("link still present")
            .file_type()
            .is_symlink(),
        "the link is untouched"
    );
    assert_eq!(fs::read(&target).expect("read target"), VICTIM_BYTES);
}

#[cfg(unix)]
#[test]
fn off_unlinks_a_symlinked_project_db_without_following_it() {
    let (project, path, link, target) = project_with_symlinked_database();

    let response = project_control(
        &request(&path, ProjectControlAction::Off),
        &crate::progress::NoopProgressSink,
        None,
    )
    .expect("off unlinks the link itself");

    assert_eq!(response.0.status, "disabled");
    assert_eq!(
        response.0.removed.len(),
        2,
        "the link and then the emptied directory: {:?}",
        response.0.removed
    );
    assert_eq!(response.0.removed[0], link.to_string_lossy());
    assert!(
        matches!(
            fs::symlink_metadata(&link),
            Err(ref error) if error.kind() == std::io::ErrorKind::NotFound
        ),
        "the link is gone"
    );
    assert_eq!(
        fs::read(&target).expect("the target survives"),
        VICTIM_BYTES,
        "off never touches what the link pointed at"
    );
    assert!(
        !project.path().join(".planning/slugaudit").exists(),
        "an emptied activation directory is removed"
    );
}

/// A refused `off` (the database is busy) leaves the project active, so its
/// watcher must still be registered; a successful retry then removes it.
#[test]
fn a_refused_off_keeps_the_watcher_and_a_later_off_removes_it() {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join("src")).expect("create src dir");
    fs::write(project.path().join("src/lib.rs"), b"pub fn a() {}\n").expect("write source");
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::with_watcher();
    project_control(
        &request(&path, ProjectControlAction::On),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("on");
    manager
        .ensure_current(&path, &crate::progress::NoopProgressSink)
        .expect("sync registers the watcher");
    assert!(
        manager.watch_state_for(project.path()).is_some(),
        "the project is watched while active"
    );
    let database = project.path().join(".planning/slugaudit/project.db");
    let writer = store::open_read_write(&database).expect("open writer");
    writer
        .execute_batch("CREATE TABLE wal_probe(x); INSERT INTO wal_probe VALUES (1);")
        .expect("leave frames in the WAL");
    let reader = store::open_read_write(&database).expect("open reader");
    reader.execute_batch("BEGIN").expect("begin read");
    let _: i64 = reader
        .query_row("SELECT count(*) FROM wal_probe", [], |row| row.get(0))
        .expect("pin a read snapshot");

    let error = project_control(
        &request(&path, ProjectControlAction::Off),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .err()
    .expect("off is refused while a reader pins the WAL");

    assert!(
        error
            .message
            .starts_with("disable: could not safely disable"),
        "{}",
        error.message
    );
    assert!(database.exists(), "the project is still active");
    assert!(
        manager.watch_state_for(project.path()).is_some(),
        "a refused off must not drop the watcher"
    );

    drop(reader);
    drop(writer);
    project_control(
        &request(&path, ProjectControlAction::Off),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("off succeeds once the reader is gone");
    assert!(
        manager.watch_state_for(project.path()).is_none(),
        "a successful off removes the watcher"
    );
}
