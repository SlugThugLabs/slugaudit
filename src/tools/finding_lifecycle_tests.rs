//! The lifetime of the session findings: each SlugAudit process keeps them in
//! its own `findings-<id>.db`, they survive a rebuild of `project.db`, and
//! they never outlive the process that wrote them or touch another live
//! process's findings.
use super::*;
use crate::progress::NoopProgressSink;
use crate::project::{self, ProjectRoot};
use crate::sync::SourceSyncManager;
use crate::tools::test_support::activated_project;
use crate::tools::test_support::own_findings_path;
use crate::tools::{
    FindingClearRequest, FindingReadRequest, LegacyPaging, ProjectControlAction,
    ProjectControlRequest, finding_clear, finding_read, project_control,
};
use rmcp::handler::server::wrapper::Parameters;
use slugaudit_test_check::{Check, TestResult};

fn activation(project: &tempfile::TempDir) -> std::path::PathBuf {
    project.path().join(".planning/slugaudit")
}

fn request(project: &tempfile::TempDir) -> FindingRequest {
    FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: "lib.rs".to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "low".to_owned(),
        category: "test".to_owned(),
        title: "lifecycle".to_owned(),
        description: "d".to_owned(),
    }
}

fn write(project: &tempfile::TempDir, manager: &SourceSyncManager) -> TestResult<i64> {
    Ok(
        finding(&Parameters(request(project)), &NoopProgressSink, manager)
            .check("finding succeeds")?
            .0
            .id,
    )
}

fn write_titled(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    title: &str,
) -> TestResult {
    let mut titled = request(project);
    titled.title = title.to_owned();
    finding(&Parameters(titled), &NoopProgressSink, manager).check("finding succeeds")?;
    Ok(())
}

fn titles(project: &tempfile::TempDir, manager: &SourceSyncManager) -> TestResult<Vec<String>> {
    Ok(read(project, manager)?
        .into_iter()
        .map(|entry| entry.title)
        .collect())
}

fn root_of(project: &tempfile::TempDir) -> TestResult<ProjectRoot> {
    ProjectRoot::resolve(project.path()).check("project root")
}

fn read(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
) -> TestResult<Vec<crate::tools::FindingReadEntry>> {
    Ok(finding_read(
        &Parameters(FindingReadRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            file: None,
            cursor: None,
            legacy_paging: LegacyPaging::default(),
        }),
        &NoopProgressSink,
        manager,
    )
    .check("finding_read succeeds")?
    .0
    .findings)
}

/// What a fresh SlugAudit process does at its first touch of the project.
fn restart_process(project: &tempfile::TempDir) -> TestResult {
    let root = ProjectRoot::resolve(project.path()).check("project root")?;
    project::forget_root_for_test(&root).check("forget the session")?;
    Ok(())
}

#[test]
fn a_finding_survives_a_project_db_rebuild_in_the_same_process() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    let id = write(&project, &manager)?;
    let database = activation(&project).join("project.db");
    let own_file = own_findings_path(&project)?;
    crate::store::remove_database_files(&database).check("delete the scratch database")?;

    let after_delete = read(&project, &manager)?;

    assert_eq!(
        after_delete.len(),
        1,
        "project.db was rebuilt, the findings file kept"
    );
    assert_eq!(own_findings_path(&project)?, own_file, "same file");
    assert_eq!(after_delete[0].id, id);
    assert_eq!(after_delete[0].status, "current");

    rusqlite::Connection::open(&database)
        .check("raw open")?
        .execute_batch("PRAGMA user_version = 999;")
        .check("foreign schema version")?;

    let after_version_bump = read(&project, &manager)?;

    assert_eq!(after_version_bump.len(), 1);
    assert_eq!(after_version_bump[0].id, id);
    assert_eq!(after_version_bump[0].status, "current");
    Ok(())
}

#[test]
fn a_dead_sessions_file_is_deleted_by_the_next_sessions_first_call() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let first_process = SourceSyncManager::default();
    write(&project, &first_process)?;
    let dead_file = own_findings_path(&project)?;
    assert!(dead_file.exists());

    restart_process(&project)?;
    let second_process = SourceSyncManager::default();

    assert!(read(&project, &second_process)?.is_empty());
    assert!(!dead_file.exists(), "the previous process's file is gone");
    let entries: Vec<String> = std::fs::read_dir(activation(&project))
        .check("activation dir")?
        .map(|entry| {
            entry
                .check("entry")
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<TestResult<_>>()?;
    let leftovers: Vec<String> = entries
        .into_iter()
        .filter(|name| name.starts_with("findings-"))
        .collect();
    let dead_stem = dead_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .check("file stem")?;
    assert!(
        leftovers.iter().all(|name| !name.contains(dead_stem)),
        "no file of the dead session remains: {leftovers:?}"
    );
    assert_eq!(
        write(&project, &second_process)?,
        1,
        "the new process numbers its findings from the start"
    );
    Ok(())
}

#[test]
fn the_legacy_findings_db_and_unlocked_leftovers_are_deleted_at_the_first_touch_only() -> TestResult
{
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let legacy = activation(&project).join("findings.db");
    let orphan = activation(&project).join("findings-dead.db");
    for planted in [&legacy, &orphan] {
        let connection = crate::store::open_findings(planted).check("plant an old file")?;
        connection
            .execute(
                "INSERT INTO findings (path, source_hash, line_start, line_end, severity, \
                 category, title, description, created_at_unix, evidence_revision) \
                 VALUES ('lib.rs', 'h', 1, 1, 'low', 'c', 'old conclusion', 'd', 0, 'r')",
                [],
            )
            .check("old finding")?;
    }
    let manager = SourceSyncManager::default();

    assert!(
        read(&project, &manager)?.is_empty(),
        "the first touch wipes"
    );
    assert!(!legacy.exists(), "the legacy file is deleted");
    assert!(!orphan.exists(), "an unlocked session file is deleted");
    write(&project, &manager)?;
    let again = read(&project, &manager)?;

    assert_eq!(again.len(), 1, "a later call in the same process keeps it");
    assert_eq!(again[0].title, "lifecycle");
    Ok(())
}

#[test]
fn two_live_sessions_keep_their_own_findings() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let root = root_of(&project)?;
    // One manager serves both sessions: the paged-result store's own lock is
    // keyed by the process, and both sessions here are one test process.
    let manager = SourceSyncManager::default();
    write_titled(&project, &manager, "from a")?;
    let file_a = own_findings_path(&project)?;
    // Session A stays alive (its lock stays held) while session B starts.
    let held_a = project::take_session_for_test(&root)
        .check("the session table")?
        .check("session a")?;

    assert!(titles(&project, &manager)?.is_empty(), "b starts empty");
    assert!(file_a.exists(), "b's first call left a's file alone");
    write_titled(&project, &manager, "from b")?;
    let file_b = own_findings_path(&project)?;
    assert_ne!(file_a, file_b);
    assert_eq!(titles(&project, &manager)?, ["from b"]);

    let held_b = project::put_session_for_test(&root, held_a)
        .check("the session table")?
        .check("session b")?;
    assert_eq!(titles(&project, &manager)?, ["from a"]);
    assert!(file_b.exists());
    drop(held_b);
    Ok(())
}

#[test]
fn finding_clear_empties_only_the_own_sessions_file() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let root = root_of(&project)?;
    // One manager serves both sessions: the paged-result store's own lock is
    // keyed by the process, and both sessions here are one test process.
    let manager = SourceSyncManager::default();
    write_titled(&project, &manager, "from a")?;
    let held_a = project::take_session_for_test(&root)
        .check("the session table")?
        .check("session a")?;
    write_titled(&project, &manager, "from b")?;

    let cleared = finding_clear(
        &Parameters(FindingClearRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &NoopProgressSink,
        &manager,
    )
    .check("finding_clear succeeds")?;

    assert_eq!(cleared.0.removed, 1);
    assert!(titles(&project, &manager)?.is_empty());
    let held_b = project::put_session_for_test(&root, held_a)
        .check("the session table")?
        .check("session b")?;
    assert_eq!(titles(&project, &manager)?, ["from a"]);
    drop(held_b);
    Ok(())
}

#[test]
fn a_dead_file_that_cannot_be_deleted_fails_the_call_and_the_next_call_retries() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    // A directory where a session database should be: removing it as a file
    // fails.
    let blocker = activation(&project).join("findings-stuck.db");
    std::fs::create_dir(&blocker).check("plant a directory")?;
    let manager = SourceSyncManager::default();

    let Err(error) = finding_clear(
        &Parameters(FindingClearRequest {
            path: project.path().to_string_lossy().into_owned(),
        }),
        &NoopProgressSink,
        &manager,
    ) else {
        return Err("the sweep must fail the call".to_owned()).check("unexpected outcome");
    };

    assert!(
        error
            .message
            .starts_with("failed to set up the session findings files at"),
        "{}",
        error.message
    );
    std::fs::remove_dir(&blocker).check("clear the blocker")?;
    assert!(read(&project, &manager)?.is_empty(), "the retry succeeds");
    Ok(())
}

#[test]
fn off_deletes_every_findings_file_and_the_next_on_starts_a_new_session() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let root = root_of(&project)?;
    let manager = SourceSyncManager::default();
    write(&project, &manager)?;
    let file = own_findings_path(&project)?;
    // Another live session's file in the same directory.
    let other = project::take_session_for_test(&root)
        .check("the session table")?
        .check("session")?;
    let other_manager = SourceSyncManager::default();
    write_titled(&project, &other_manager, "other")?;
    let _held_other = project::put_session_for_test(&root, other)
        .check("the session table")?
        .check("replaced")?;

    project_control(
        &Parameters(ProjectControlRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            action: ProjectControlAction::Off,
            profile: None,
        }),
        &NoopProgressSink,
        Some(&manager),
    )
    .check("off succeeds")?;

    assert!(!file.exists());
    assert!(
        !activation(&project).exists(),
        "the whole directory is gone"
    );
    assert!(
        project::session_findings_path(&activation_dir_of(&root))
            .check("the session table")?
            .is_none(),
        "off ended this process's session"
    );
    Ok(())
}

fn activation_dir_of(root: &ProjectRoot) -> std::path::PathBuf {
    project::activation_dir(root)
}

#[test]
fn a_finding_written_before_project_off_does_not_return_after_on_in_a_new_process() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = SourceSyncManager::default();
    write(&project, &manager)?;

    let path = project.path().to_string_lossy().into_owned();
    for action in [ProjectControlAction::Off, ProjectControlAction::On] {
        project_control(
            &Parameters(ProjectControlRequest {
                path: Some(path.clone()),
                action,
                profile: None,
            }),
            &NoopProgressSink,
            Some(&manager),
        )
        .check("project_control succeeds")?;
    }
    restart_process(&project)?;

    assert!(read(&project, &SourceSyncManager::default())?.is_empty());
    Ok(())
}
