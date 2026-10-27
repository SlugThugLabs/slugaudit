//! Phase 9 golden responses for `structure`; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{StructureRequest, structure};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;

fn structure_request(project: &tempfile::TempDir, file: Option<&str>) -> StructureRequest {
    StructureRequest {
        path: path_of(project),
        file: file.map(str::to_owned),
        language: file.is_none().then(|| "rust".to_owned()),
        query: "(function_item name: (identifier) @name)".to_owned(),
        ..Default::default()
    }
}

#[test]
fn golden_phase9_structure_single_file_and_project_wide() {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    for (name, file) in [
        ("structure_single_file", Some("mod0.rs")),
        ("structure_project_wide", None),
    ] {
        let request = structure_request(&project, file);
        let response = structure(
            &Parameters(structure_request(&project, file)),
            &NoopProgressSink,
            &manager,
        )
        .expect("structure succeeds")
        .0;
        check_case(
            name,
            &project,
            &json!({ "file": request.file, "language": request.language }),
            &response,
        );
    }
}
