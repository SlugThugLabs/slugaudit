//! Phase 9 golden responses for `mechanic` add and list; see `golden_phase9_support`.
use super::golden_phase9_support::{check_case, path_of, rust_project};
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{LegacyPaging, MechanicAction, MechanicRequest, mechanic};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;
use slugaudit_test_check::{Check, TestResult};

fn mechanic_request(
    project: &tempfile::TempDir,
    action: MechanicAction,
) -> TestResult<MechanicRequest> {
    Ok(MechanicRequest {
        path: Some(path_of(project)?),
        cursor: None,
        legacy_paging: LegacyPaging::default(),
        action,
        id: None,
        language: None,
        query: None,
        note: None,
        limit: None,
        page_bytes: None,
    })
}

fn add_println_mechanic(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
) -> TestResult<(MechanicRequest, Value)> {
    let mut add = mechanic_request(project, MechanicAction::Add)?;
    add.id = Some("println_calls".to_owned());
    add.language = Some("rust".to_owned());
    add.query = Some(r#"(macro_invocation macro: (identifier) @m (#eq? @m "println"))"#.to_owned());
    add.note = Some("println macro calls".to_owned());
    add.limit = Some(2);
    let response = mechanic(
        &Parameters(clone_mechanic(&add)),
        &NoopProgressSink,
        manager,
    )
    .check("mechanic add succeeds")?
    .0;
    Ok((
        add,
        serde_json::to_value(response).check("response serializes")?,
    ))
}

fn clone_mechanic(request: &MechanicRequest) -> MechanicRequest {
    MechanicRequest {
        path: request.path.clone(),
        cursor: request.cursor.clone(),
        legacy_paging: LegacyPaging::default(),
        action: request.action,
        id: request.id.clone(),
        language: request.language.clone(),
        query: request.query.clone(),
        note: request.note.clone(),
        limit: request.limit,
        page_bytes: request.page_bytes,
    }
}

#[test]
fn golden_phase9_mechanic_add_and_list() -> TestResult {
    let project = rust_project()?;
    let manager = SourceSyncManager::default();

    let (add, added) = add_println_mechanic(&project, &manager)?;
    check_case("mechanic_add", &project, &add.id, &added)?;
    let list = mechanic_request(&project, MechanicAction::List)?;
    let listed = mechanic(
        &Parameters(clone_mechanic(&list)),
        &NoopProgressSink,
        &manager,
    )
    .check("mechanic list succeeds")?
    .0;
    let page = listed.page.clone().check("list carries a page block")?;
    assert_eq!(
        (page.returned, page.complete, page.cursor),
        (listed.mechanics.len(), true, None),
        "the small catalogue fits one page"
    );
    // The fixture holds no `page` block, which `list` carries with the cursor; the
    // block is asserted above, everything else is compared unchanged.
    let mut listed = serde_json::to_value(listed).check("response serializes")?;
    listed
        .as_object_mut()
        .check("response is an object")?
        .remove("page");
    check_case("mechanic_list", &project, &"list", &listed)?;
    Ok(())
}
