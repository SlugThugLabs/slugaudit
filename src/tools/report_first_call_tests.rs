//! The first `report` after `project_control on` answers at the revision
//! `on` published, and a change that lands after a call's scan is served by
//! the next call — never retroactively.
use super::*;
use rmcp::handler::server::wrapper::Parameters;

/// The first state-bearing call after a server starts syncs the whole tree
/// and publishes nothing when the tree matches, so it reads the revision
/// `on` returned.
#[test]
fn the_first_report_after_project_control_on_succeeds_at_the_imported_revision() {
    let project = crate::tools::test_support::activated_project("lib.rs", b"pub fn a() {}\n");
    std::fs::write(project.path().join("tool.py"), b"def b():\n    return 1\n")
        .expect("write a second source file");
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::new();
    let enabled = crate::tools::project_control(
        &Parameters(crate::tools::ProjectControlRequest {
            path: Some(path.clone()),
            action: crate::tools::ProjectControlAction::On,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("on enables the project");
    let imported = enabled
        .0
        .import
        .expect("on runs the initial import")
        .revision_id;

    let report = report(
        &Parameters(ReportRequest { path }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("the first report after on succeeds");

    assert_eq!(report.0.revision_id, imported);
}

/// Writes a change into the project once, right after the publish commits.
struct WriteAfterPublish {
    file: std::path::PathBuf,
    written: std::sync::atomic::AtomicBool,
}

impl crate::progress::ProgressSink for WriteAfterPublish {
    fn emit(&self, event: crate::progress::ProgressEvent) {
        let published = matches!(
            event,
            crate::progress::ProgressEvent::Completed {
                phase: "publishing"
            }
        );
        if published && !self.written.swap(true, std::sync::atomic::Ordering::SeqCst) {
            std::fs::write(&self.file, b"pub fn a() {}\npub fn b() {}\n")
                .expect("change the file mid-call");
        }
    }
}

/// The contract is "matches at the scan instant": a write that lands after
/// the scan is answered by the next call, and the call that raced it answers
/// from the revision it scanned.
#[test]
fn a_change_after_the_scan_is_served_on_the_next_call() {
    let project = crate::tools::test_support::activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::new();
    crate::tools::project_control(
        &Parameters(crate::tools::ProjectControlRequest {
            path: Some(path.clone()),
            action: crate::tools::ProjectControlAction::On,
            profile: None,
        }),
        &crate::progress::NoopProgressSink,
        Some(&manager),
    )
    .expect("on enables the project");
    let sink = WriteAfterPublish {
        file: project.path().join("lib.rs"),
        written: std::sync::atomic::AtomicBool::new(false),
    };

    let first = report(
        &Parameters(ReportRequest { path: path.clone() }),
        &sink,
        &manager,
    )
    .expect("the racing call answers from its scan");
    assert!(
        sink.written.load(std::sync::atomic::Ordering::SeqCst),
        "the sink wrote during the call"
    );

    let second = report(
        &Parameters(ReportRequest { path }),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .expect("the next call syncs again");

    assert_eq!(
        first.0.revision_id, "rev-1",
        "the scan that answered the first call saw the original file"
    );
    assert_eq!(
        second.0.revision_id, "rev-2",
        "the next call observes the change the first scan missed"
    );
}
