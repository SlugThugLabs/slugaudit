//! `mechanic`: add -> count -> remove, collisions, refused queries, and the
//! order `leads` and `report` list registered entries in. Each
//! `SourceSyncManager` is one server instance.
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::{LeadMode, LeadRows, LeadsRequest, LeadsResponse, ReportRequest};
use std::fs;

pub(super) const EVAL_QUERY: &str =
    r#"(call function: (identifier) @callee (#eq? @callee "eval"))"#;

/// `eval` is called on lines 4, 6 and 8 of `app.py` and line 1 of
/// `lib/util.py`; the comment and the string on lines 4 and 5 are decoys.
const APP_PY: &str = "import os

def run(code):
    eval(code)  # eval(comment)
    text = \"eval(string)\"
    return eval(code + text)

eval(\"2\")
";

pub(super) fn python_project() -> tempfile::TempDir {
    let project = crate::tools::test_support::activated_project("app.py", APP_PY.as_bytes());
    fs::create_dir_all(project.path().join("lib")).expect("create lib dir");
    fs::write(project.path().join("lib").join("util.py"), "eval(\"3\")\n")
        .expect("write second python file");
    project
}

pub(super) fn path_of(project: &tempfile::TempDir) -> String {
    project.path().to_str().expect("utf-8 path").to_owned()
}

pub(super) fn add(path: &str, id: &str, language: &str, query: &str) -> MechanicRequest {
    MechanicRequest {
        path: Some(path.to_owned()),
        cursor: None,
        legacy_paging: LegacyPaging::default(),
        action: MechanicAction::Add,
        id: Some(id.to_owned()),
        index: None,
        language: Some(language.to_owned()),
        query: Some(query.to_owned()),
        note: Some("dynamic code execution".to_owned()),
        persist: false,
        limit: None,
        page_bytes: None,
    }
}

pub(super) fn action(path: &str, action: MechanicAction, id: Option<&str>) -> MechanicRequest {
    MechanicRequest {
        path: Some(path.to_owned()),
        cursor: None,
        legacy_paging: LegacyPaging::default(),
        action,
        id: id.map(str::to_owned),
        index: None,
        language: None,
        query: None,
        note: None,
        persist: false,
        limit: None,
        page_bytes: None,
    }
}

pub(super) fn call(
    manager: &sync::SourceSyncManager,
    request: MechanicRequest,
) -> Result<MechanicResponse, ErrorData> {
    mechanic(&Parameters(request), &NoopProgressSink, manager).map(|response| response.0)
}

pub(super) fn leads_request(path: &str, mechanic: Option<&str>, mode: LeadMode) -> LeadsRequest {
    LeadsRequest {
        path: path.to_owned(),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        offset: None,
        expected_revision: None,
        mode,
        page_bytes: None,
    }
}

pub(super) fn run_leads(manager: &sync::SourceSyncManager, request: LeadsRequest) -> LeadsResponse {
    crate::tools::leads(&Parameters(request), &NoopProgressSink, manager)
        .expect("leads succeeds")
        .0
}

pub(super) fn report_of(
    manager: &sync::SourceSyncManager,
    path: &str,
) -> crate::tools::ReportResponse {
    let request = ReportRequest {
        path: path.to_owned(),
    };
    crate::tools::report(&Parameters(request), &NoopProgressSink, manager)
        .expect("report succeeds")
        .0
}

pub(super) fn sources(response: &LeadsResponse) -> Vec<(String, MechanicSource)> {
    response
        .mechanic_counts
        .iter()
        .map(|count| (count.mechanic.clone(), count.source))
        .collect()
}

pub(super) fn refusal(manager: &sync::SourceSyncManager, request: MechanicRequest) -> ErrorData {
    match call(manager, request) {
        Ok(response) => panic!("the call must be refused, got {response:?}"),
        Err(error) => error,
    }
}

pub(super) fn assert_refused(error: &ErrorData, needle: &str) {
    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{error:?}"
    );
    assert!(
        error.message.contains(needle),
        "the message names the reason `{needle}`: {}",
        error.message
    );
}

#[test]
fn adding_a_session_mechanic_returns_its_first_count_and_remove_drops_it() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let added = call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");

    assert_eq!(added.source, Some(MechanicSource::Session));
    let first = added.first_count.expect("add carries the first count");
    let entry = first
        .mechanic_counts
        .first()
        .expect("the session entry is listed first");
    assert_eq!(
        (entry.mechanic.as_str(), entry.count, entry.files),
        ("py_eval", 4, 2)
    );
    assert_eq!(entry.covers, ["python"]);
    assert_eq!(entry.note.as_deref(), Some("dynamic code execution"));
    assert_eq!(
        (first.page.total, first.files_total, first.files_covered),
        (Some(4), 2, 2)
    );
    assert_eq!(first.files_not_parsed, 0);
    let LeadRows::Full(rows) = &first.rows else {
        panic!("a full page");
    };
    let lines: Vec<(&str, i64)> = rows
        .iter()
        .map(|row| (row.file.as_str(), row.line))
        .collect();
    assert_eq!(
        lines,
        [
            ("app.py", 4),
            ("app.py", 6),
            ("app.py", 8),
            ("lib/util.py", 1)
        ]
    );
    assert_eq!(rows[0].detail, "eval");

    let removed = call(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    )
    .expect("remove");

    assert_eq!(removed.source, Some(MechanicSource::Session));
    let after = run_leads(&manager, leads_request(&path, None, LeadMode::Full));
    assert!(
        after
            .mechanic_counts
            .iter()
            .all(|count| count.mechanic != "py_eval"),
        "a removed entry is no longer counted"
    );
    let again = refusal(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    );
    assert_refused(&again, "no session or persisted mechanic has id `py_eval`");
    let code = again.data.as_ref().and_then(|data| data.get("code"));
    assert_eq!(
        code.and_then(|code| code.as_str()),
        Some("unknown_mechanic")
    );
}

#[test]
fn a_registered_mechanic_pages_by_limit_and_offset_with_cumulative_files() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let mut request = leads_request(&path, Some("py_eval"), LeadMode::Compact);
    request.limit = Some(3);

    let first = run_leads(&manager, request);

    assert_eq!(
        (first.page.returned, first.page.total, first.page.complete),
        (3, Some(4), false)
    );
    assert_eq!(first.page.limited_by, crate::tools::PageLimit::Limit);
    assert_eq!((first.files_total, first.files_covered), (2, 1));
    assert_eq!(first.page.next_offset, Some(3));
    let mut next = leads_request(&path, Some("py_eval"), LeadMode::Compact);
    next.offset = first.page.next_offset;
    next.expected_revision = Some(first.page.revision_id.clone());
    let second = run_leads(&manager, next);
    assert_eq!((second.page.returned, second.page.complete), (1, true));
    assert_eq!(second.files_covered, 2);
    let LeadRows::Compact(rows) = &second.rows else {
        panic!("a compact page");
    };
    assert_eq!(
        (
            rows[0].mechanic.as_str(),
            rows[0].file.as_str(),
            rows[0].line
        ),
        ("py_eval", "lib/util.py", 1)
    );
}

#[test]
fn a_compact_whole_surface_page_lists_registered_rows_before_builtin_rows() {
    let project = python_project();
    fs::write(project.path().join("todo.py"), "# TODO: one\n").expect("write marker file");
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("add");
    let mut request = leads_request(&path, None, LeadMode::Compact);
    request.limit = Some(4);

    let first = run_leads(&manager, request);

    assert_eq!((first.page.returned, first.page.total), (4, Some(5)));
    assert_eq!((first.files_total, first.files_covered), (3, 2));
    let mut next = leads_request(&path, None, LeadMode::Compact);
    next.offset = first.page.next_offset;
    let second = run_leads(&manager, next);
    let LeadRows::Compact(rows) = &second.rows else {
        panic!("a compact page");
    };
    let tail: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.mechanic.as_str(), row.file.as_str()))
        .collect();
    assert_eq!(tail, [("comment_markers", "todo.py")]);
    assert_eq!((second.page.complete, second.files_covered), (true, 3));
}

#[test]
fn adding_the_id_of_a_builtin_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "unwrap_calls", "python", EVAL_QUERY));

    assert_refused(&error, "can never remove, rename or override a built-in");
}

#[test]
fn adding_an_id_already_registered_for_the_session_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("first add");

    let mut persisted = add(&path, "py_eval", "python", EVAL_QUERY);
    persisted.persist = true;
    let error = refusal(&manager, persisted);

    assert_refused(&error, "already registered for this session");
    assert!(
        !project
            .path()
            .join(".planning/slugaudit/config.json")
            .exists(),
        "a refused persist writes nothing"
    );
}

#[test]
fn adding_an_id_already_persisted_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    let mut persisted = add(&path, "py_eval", "python", EVAL_QUERY);
    persisted.persist = true;
    call(&manager, persisted).expect("persist");

    let error = refusal(&manager, add(&path, "py_eval", "python", EVAL_QUERY));

    assert_refused(&error, "already persisted in config.json");
}

#[test]
fn a_query_that_does_not_compile_is_refused_with_the_compiler_message() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "broken", "python", "(call @callee"));

    assert_refused(&error, "query does not compile");
}

#[test]
fn a_query_with_two_captures_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    let query = "(call function: (identifier) @callee arguments: (argument_list) @args)";

    let error = refusal(&manager, add(&path, "two", "python", query));

    assert_refused(&error, "exactly one capture; it names 2");
}

#[test]
fn a_query_with_no_capture_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "none", "python", "(call)"));

    assert_refused(&error, "exactly one capture; it names 0");
}

#[test]
fn a_mechanic_on_a_prose_language_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "docs", "markdown", "(paragraph) @p"));

    assert_refused(&error, "is prose");
}

#[test]
fn a_mechanic_on_an_unknown_language_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "nope", "no_such_language", "(x) @x"));

    assert_refused(&error, "unknown language `no_such_language`");
}

#[test]
fn an_id_outside_the_charset_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "Py-Eval", "python", EVAL_QUERY));

    assert_refused(&error, "a-z, 0-9 and _");
}

#[test]
fn removing_a_builtin_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(
        &manager,
        action(&path, MechanicAction::Remove, Some("diagnostics")),
    );

    assert_refused(&error, "built-ins can never be removed");
}

#[test]
fn the_mechanics_note_is_on_the_first_report_of_a_server_only() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let first = report_of(&manager, &path);
    let second = report_of(&manager, &path);
    let other_server = report_of(&sync::SourceSyncManager::default(), &path);

    let note = first
        .mechanics
        .note
        .expect("the first report carries the note");
    assert!(
        note.contains("counting primitives, not a checklist"),
        "{note}"
    );
    assert!(note.contains("`persist: true`"), "{note}");
    assert_eq!(
        second.mechanics.note, None,
        "only the first report carries it"
    );
    assert!(
        other_server.mechanics.note.is_some(),
        "a new server instance starts a new session"
    );
}

#[test]
fn leads_and_report_list_session_then_project_config_then_builtin() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    let mut persisted = add(
        &path,
        "py_exec",
        "python",
        &EVAL_QUERY.replace("eval", "exec"),
    );
    persisted.persist = true;
    call(&manager, persisted).expect("persist");
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).expect("session add");

    let leads = run_leads(&manager, leads_request(&path, None, LeadMode::Full));
    let report = report_of(&manager, &path);

    let listed = sources(&leads);
    assert_eq!(listed[0], ("py_eval".to_owned(), MechanicSource::Session));
    assert_eq!(
        listed[1],
        ("py_exec".to_owned(), MechanicSource::ProjectConfig)
    );
    assert!(
        listed[2..]
            .iter()
            .all(|(_, source)| *source == MechanicSource::Builtin),
        "{listed:?}"
    );
    assert_eq!(
        listed.len(),
        2 + crate::tools::leads_mechanics::MECHANICS.len()
    );
    let reported: Vec<(&str, MechanicSource)> = report
        .mechanics
        .entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry.source))
        .collect();
    let expected: Vec<(&str, MechanicSource)> = listed
        .iter()
        .map(|(id, source)| (id.as_str(), *source))
        .collect();
    assert_eq!(reported, expected, "report lists the same order as leads");
}

#[test]
fn a_capitalised_language_is_refused_with_the_naming_convention() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "py_eval", "Python", EVAL_QUERY));

    assert_refused(&error, "lowercase pack name the index stores");
}

#[test]
fn a_pack_alias_language_is_refused_with_its_canonical_name() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "sh_eval", "shell", "(command) @c"));

    assert_refused(&error, "use `bash`");
}

#[test]
fn a_remove_with_both_id_and_index_is_refused() {
    let project = python_project();
    let path = path_of(&project);
    let manager = sync::SourceSyncManager::default();
    let mut request = action(&path, MechanicAction::Remove, Some("py_eval"));
    request.index = Some(0);

    let error = refusal(&manager, request);

    assert_refused(&error, "`id` or `index`, not both");
}
