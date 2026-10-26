//! The first `report` after `project_control on`, on the production manager
//! shape (a real watcher), answers at the revision `on` published.
use super::*;
use rmcp::handler::server::wrapper::Parameters;

/// The deep dive saw this pair fail with `stale_revision` (expected rev-N,
/// now rev-N+1). The first state-bearing call after a server starts finds
/// the watcher untrusted and runs a full verification publish followed by a
/// drain of what arrived meanwhile; on a project nothing writes to during
/// that call, neither step publishes, so the call reads the revision `on`
/// returned.
#[test]
fn the_first_report_after_project_control_on_succeeds_at_the_imported_revision() {
    let project = crate::tools::test_support::activated_project("lib.rs", b"pub fn a() {}\n");
    std::fs::write(project.path().join("tool.py"), b"def b():\n    return 1\n")
        .expect("write a second source file");
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::with_watcher();
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

/// Writes a change into the project once, right after the first full
/// publish commits, so the drain that follows it inside the same call finds
/// a change the publish did not see.
struct WriteAfterFirstPublish {
    file: std::path::PathBuf,
    written: std::sync::atomic::AtomicBool,
}

impl crate::progress::ProgressSink for WriteAfterFirstPublish {
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

/// The mechanism behind the deep-dive repro: a file that changes while the
/// first call's verification publish runs is picked up by the drain, which
/// publishes again. The call must answer at that newer revision rather than
/// pin the one the drain superseded and refuse itself with `stale_revision`.
#[test]
fn a_change_during_the_first_full_publish_is_served_not_refused_as_stale() {
    let project = crate::tools::test_support::activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let manager = sync::SourceSyncManager::with_watcher();
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
    let sink = WriteAfterFirstPublish {
        file: project.path().join("lib.rs"),
        written: std::sync::atomic::AtomicBool::new(false),
    };

    let report = report(&Parameters(ReportRequest { path }), &sink, &manager)
        .expect("a change during the first call is served, not refused");

    assert_eq!(
        report.0.revision_id, "rev-2",
        "the drain's revision is served"
    );
    assert!(sink.written.load(std::sync::atomic::Ordering::SeqCst));
}
