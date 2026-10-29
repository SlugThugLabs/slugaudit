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
