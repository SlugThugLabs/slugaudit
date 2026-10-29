//! The window between `ensure_current`'s activation check and its database
//! open: `project_control off` landing in it must leave the project off, not
//! have the in-flight call recreate `project.db` and re-import.

use crate::error_codes::{self, ErrorCode};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::sync::race_hook;
use crate::sync::test_support::{create_project, sync_project, write_file};

#[test]
fn an_off_landing_after_the_activation_check_leaves_the_project_off() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);
    // A kept `config.json` keeps the activation directory, so a recreated
    // database would succeed: the failure mode this test exists to catch.
    write_file(&project, ".planning/slugaudit/config.json", b"{}\n");
    let database = project.path().join(".planning/slugaudit/project.db");
    assert!(database.exists(), "the project starts active");

    // The hook (keyed by the database path) runs inside `ensure_current`
    // right after `resolve_project` passed, so `off` runs exactly in the window under test.
    let hook_root = project.path().to_path_buf();
    race_hook::set(&database, move || {
        let root = crate::project::ProjectRoot::resolve(&hook_root).expect("resolve root");
        let report = crate::project::disable(&root).expect("off inside the window");
        assert!(
            report
                .removed
                .iter()
                .any(|path| path.ends_with("project.db")),
            "off removed the database: {report:?}"
        );
    });

    let error = manager
        .ensure_current(&project.path().to_string_lossy(), &NoopProgressSink)
        .err()
        .expect("a call that lost the race to off must fail");

    assert_eq!(
        error.message,
        "SlugAudit is not enabled for this project or any parent directory"
    );
    assert_eq!(
        error.data,
        error_codes::payload(ErrorCode::ProjectNotEnabled),
        "the failure carries the project_not_enabled code"
    );
    assert!(
        !database.exists(),
        "the in-flight call must not recreate project.db"
    );
    let after = manager
        .ensure_current(&project.path().to_string_lossy(), &NoopProgressSink)
        .err()
        .expect("the project stays off");
    assert_eq!(error.message, after.message);
    assert!(
        !database.exists(),
        "a later call does not rebuild it either"
    );
}

/// Arms `remove_database` for the window after `ensure_current` decided to
/// discard the database and before it does, then asserts the call fails as not-enabled and
/// that nothing recreated `project.db`.
fn assert_off_before_discard_leaves_project_off(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    remove_database: impl FnOnce() + Send + 'static,
) {
    let database = project.path().join(".planning/slugaudit/project.db");
    // The discard window is keyed by the activation directory.
    race_hook::set(database.parent().expect("activation dir"), remove_database);

    let error = manager
        .ensure_current(&project.path().to_string_lossy(), &NoopProgressSink)
        .err()
        .expect("a call that lost the race to off must fail");

    assert_eq!(
        error.message,
        "SlugAudit is not enabled for this project or any parent directory"
    );
    assert_eq!(
        error.data,
        error_codes::payload(ErrorCode::ProjectNotEnabled)
    );
    assert!(
        !database.exists(),
        "the in-flight call must not rebuild project.db after off removed it"
    );
}

#[test]
fn an_off_landing_between_a_corrupt_open_and_its_discard_leaves_the_project_off() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);
    let database = project.path().join(".planning/slugaudit/project.db");
    std::fs::write(&database, vec![0x5a_u8; 4096]).expect("corrupt the database");

    let doomed = database.clone();
    assert_off_before_discard_leaves_project_off(&project, &SourceSyncManager::new(), move || {
        // `off` cannot lock a file that is not a database, so the corrupt
        // file is removed directly: the state `off` leaves behind.
        for suffix in ["", "-wal", "-shm"] {
            let mut name = doomed.as_os_str().to_os_string();
            name.push(suffix);
            match std::fs::remove_file(std::path::PathBuf::from(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("removing the corrupt database: {error}"),
            }
        }
    });
}

#[test]
fn an_off_landing_between_a_stale_root_check_and_its_discard_leaves_the_project_off() {
    let manager = SourceSyncManager::new();
    let project = create_project();
    write_file(&project, "lib.rs", b"pub fn a() {}\n");
    sync_project(&manager, &project);
    let database = project.path().join(".planning/slugaudit/project.db");
    let connection = rusqlite::Connection::open(&database).expect("open database");
    connection
        .execute(
            "UPDATE project SET root_path = '/somewhere/else' WHERE id = 1",
            [],
        )
        .expect("make the stored root stale");
    drop(connection);

    let hook_root = project.path().to_path_buf();
    assert_off_before_discard_leaves_project_off(&project, &SourceSyncManager::new(), move || {
        let root = crate::project::ProjectRoot::resolve(&hook_root).expect("resolve root");
        crate::project::disable(&root).expect("off inside the discard window");
    });
}
