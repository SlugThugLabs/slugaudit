//! `mechanic`: add -> count -> remove, collisions, refused queries, and the
//! order `leads` and `report` list registered entries in. Each
//! `SourceSyncManager` is one server instance.
use super::*;
use crate::progress::NoopProgressSink;
use crate::tools::{LeadMode, LeadRows, LeadsRequest, LeadsResponse, ReportRequest};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
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

pub(super) fn python_project() -> TestResult<tempfile::TempDir> {
    let project = crate::tools::test_support::activated_project("app.py", APP_PY.as_bytes())?;
    fs::create_dir_all(project.path().join("lib")).check("create lib dir")?;
    fs::write(project.path().join("lib").join("util.py"), "eval(\"3\")\n")
        .check("write second python file")?;
    Ok(project)
}

pub(super) fn path_of(project: &tempfile::TempDir) -> TestResult<String> {
    Ok(project.path().to_str().check("utf-8 path")?.to_owned())
}

pub(super) fn add(path: &str, id: &str, language: &str, query: &str) -> MechanicRequest {
    MechanicRequest {
        path: Some(path.to_owned()),
        cursor: None,
        legacy_paging: LegacyPaging::default(),
        action: MechanicAction::Add,
        id: Some(id.to_owned()),
        language: Some(language.to_owned()),
        query: Some(query.to_owned()),
        note: Some("dynamic code execution".to_owned()),
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
        language: None,
        query: None,
        note: None,
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
        path: Some(path.to_owned()),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        cursor: None,
        mode,
        page_bytes: None,
        legacy_paging: crate::tools::LegacyPaging::default(),
    }
}

pub(super) fn run_leads(
    manager: &sync::SourceSyncManager,
    request: LeadsRequest,
) -> TestResult<LeadsResponse> {
    Ok(
        crate::tools::leads(&Parameters(request), &NoopProgressSink, manager)
            .check("leads succeeds")?
            .0,
    )
}

pub(super) fn report_of(
    manager: &sync::SourceSyncManager,
    path: &str,
) -> TestResult<crate::tools::ReportResponse> {
    let request = ReportRequest {
        path: path.to_owned(),
    };
    Ok(
        crate::tools::report(&Parameters(request), &NoopProgressSink, manager)
            .check("report succeeds")?
            .0,
    )
}

pub(super) fn sources(response: &LeadsResponse) -> Vec<(String, MechanicSource)> {
    response
        .mechanic_counts
        .iter()
        .map(|count| (count.mechanic.clone(), count.source))
        .collect()
}

pub(super) fn refusal(
    manager: &sync::SourceSyncManager,
    request: MechanicRequest,
) -> TestResult<ErrorData> {
    call(manager, request).check_err("the call must be refused")
}

pub(super) fn assert_refused(error: &ErrorData, needle: &str) -> TestResult {
    ensure_eq(
        &error.code,
        &rmcp::model::ErrorCode::INVALID_PARAMS,
        "error.code == rmcp::model::ErrorCode::INVALID_PARAMS",
    )?;
    ensure(
        error.message.contains(needle),
        format!("the message names the reason `{needle}`: {}", error.message),
    )?;

    Ok(())
}

#[test]
fn adding_a_session_mechanic_returns_its_first_count_and_remove_drops_it() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let added = call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;

    ensure_eq(
        &added.source,
        &Some(MechanicSource::Session),
        "added.source == Some(MechanicSource::Session)",
    )?;
    let first = added.first_count.check("add carries the first count")?;
    let entry = first
        .mechanic_counts
        .first()
        .check("the session entry is listed first")?;
    ensure_eq(
        &(entry.mechanic.as_str(), entry.count, entry.files),
        &("py_eval", 4, 2),
        "(entry.mechanic.as_str(), entry.count, entry.files) == (\"py_eval\", 4, 2)",
    )?;
    ensure_eq(&entry.covers, &["python"], "entry.covers == [\"python\"]")?;
    ensure_eq(
        &entry.note.as_deref(),
        &Some("dynamic code execution"),
        "entry.note.as_deref() == Some(\"dynamic code execution\")",
    )?;
    ensure_eq(
        &(first.page.total, first.files_total, first.files_covered),
        &(Some(4), 2, 2),
        "(first.page.total, first.files_total, first.files_covered) == (Some(4), 2, 2)",
    )?;
    ensure_eq(&first.files_not_parsed, &0, "first.files_not_parsed == 0")?;
    let LeadRows::Full(rows) = &first.rows else {
        return Err("a full page".to_owned()).check("unexpected outcome");
    };
    let lines: Vec<(&str, i64)> = rows
        .iter()
        .map(|row| (row.file.as_str(), row.line))
        .collect();
    ensure_eq(
        &lines,
        &[
            ("app.py", 4),
            ("app.py", 6),
            ("app.py", 8),
            ("lib/util.py", 1),
        ],
        "lines == [ (\"app.py\", 4), (\"app.py\", 6), (\"app.py\", 8), (\"lib/util.py\", 1) ]",
    )?;
    ensure_eq(
        &rows.first().check("rows[0]")?.detail,
        &"eval",
        "rows[0].detail == \"eval\"",
    )?;

    let removed = call(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    )
    .check("remove")?;

    ensure_eq(
        &removed.source,
        &Some(MechanicSource::Session),
        "removed.source == Some(MechanicSource::Session)",
    )?;
    let after = run_leads(&manager, leads_request(&path, None, LeadMode::Full))?;
    ensure(
        after
            .mechanic_counts
            .iter()
            .all(|count| count.mechanic != "py_eval"),
        "after .mechanic_counts .iter() .all(|count| count.mechanic != \"py_eval\")",
    )?;
    let again = refusal(
        &manager,
        action(&path, MechanicAction::Remove, Some("py_eval")),
    )?;
    assert_refused(&again, "no registered mechanic has id `py_eval`")?;
    let code = again.data.as_ref().and_then(|data| data.get("code"));
    ensure_eq(
        &code.and_then(|code| code.as_str()),
        &Some("unknown_mechanic"),
        "code.and_then(|code| code.as_str()) == Some(\"unknown_mechanic\")",
    )?;
    Ok(())
}

#[test]
fn a_registered_mechanic_pages_by_limit_and_offset_with_cumulative_files() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let mut request = leads_request(&path, Some("py_eval"), LeadMode::Compact);
    request.limit = Some(3);

    let first = run_leads(&manager, request)?;

    ensure_eq(
        &(first.page.returned, first.page.total, first.page.complete),
        &(3, Some(4), false),
        "(first.page.returned, first.page.total, first.page.complete) == (3, Some(4), false)",
    )?;
    ensure_eq(
        &first.page.limited_by,
        &crate::tools::PageLimit::Limit,
        "first.page.limited_by == crate::tools::PageLimit::Limit",
    )?;
    ensure_eq(
        &(first.files_total, first.files_covered),
        &(2, 1),
        "(first.files_total, first.files_covered) == (2, 1)",
    )?;
    first
        .page
        .cursor
        .as_ref()
        .check("`first.page.cursor` is present")?;
    let mut next = leads_request(&path, Some("py_eval"), LeadMode::Compact);
    next.path = None;
    next.cursor = first.page.cursor.clone();
    let second = run_leads(&manager, next)?;
    ensure_eq(
        &(second.page.returned, second.page.complete),
        &(1, true),
        "(second.page.returned, second.page.complete) == (1, true)",
    )?;
    ensure_eq(&second.files_covered, &2, "second.files_covered == 2")?;
    let LeadRows::Compact(rows) = &second.rows else {
        return Err("a compact page".to_owned()).check("unexpected outcome");
    };
    ensure_eq(
        &(
            rows.first().check("rows[0]")?.mechanic.as_str(),
            rows.first().check("rows[0]")?.file.as_str(),
            rows.first().check("rows[0]")?.line,
        ),
        &("py_eval", "lib/util.py", 1),
        "( rows[0].mechanic.as_str(), rows[0].file.as_str(), rows[0].line ) == (\"py_eval\", \"lib/util.py\", 1)",
    )?;
    Ok(())
}

#[test]
fn a_compact_whole_surface_page_lists_registered_rows_before_builtin_rows() -> TestResult {
    let project = python_project()?;
    fs::write(project.path().join("todo.py"), "# TODO: one\n").check("write marker file")?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    let mut request = leads_request(&path, None, LeadMode::Compact);
    request.limit = Some(4);

    let first = run_leads(&manager, request)?;

    ensure_eq(
        &(first.page.returned, first.page.total),
        &(4, Some(5)),
        "(first.page.returned, first.page.total) == (4, Some(5))",
    )?;
    ensure_eq(
        &(first.files_total, first.files_covered),
        &(3, 2),
        "(first.files_total, first.files_covered) == (3, 2)",
    )?;
    let mut next = leads_request(&path, None, LeadMode::Compact);
    next.path = None;
    next.cursor = first.page.cursor.clone();
    let second = run_leads(&manager, next)?;
    let LeadRows::Compact(rows) = &second.rows else {
        return Err("a compact page".to_owned()).check("unexpected outcome");
    };
    let tail: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.mechanic.as_str(), row.file.as_str()))
        .collect();
    ensure_eq(
        &tail,
        &[("comment_markers", "todo.py")],
        "tail == [(\"comment_markers\", \"todo.py\")]",
    )?;
    ensure_eq(
        &(second.page.complete, second.files_covered),
        &(true, 3),
        "(second.page.complete, second.files_covered) == (true, 3)",
    )?;
    Ok(())
}

/// An added mechanic lives in memory only: no `persist` parameter exists, and
/// nothing is written under the activation directory but the two databases
/// (and their WAL sidecars) and the stored pages of the paged answer `add`
/// itself returns.
#[test]
fn add_writes_no_config_and_nothing_but_the_databases_and_stored_pages() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("add")?;
    call(&manager, action(&path, MechanicAction::List, None)).check("list")?;
    crate::tools::finding(
        &Parameters(crate::tools::FindingRequest {
            path: path.clone(),
            file: "app.py".to_owned(),
            line_start: 1,
            line_end: 1,
            severity: "low".to_owned(),
            category: "test".to_owned(),
            title: "t".to_owned(),
            description: "d".to_owned(),
        }),
        &NoopProgressSink,
        &manager,
    )
    .check("finding")?;

    let mut names: Vec<String> = fs::read_dir(project.path().join(".planning/slugaudit"))
        .check("activation dir")?
        .map(|entry| {
            entry
                .check("directory entry")
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<TestResult<_>>()?;
    names.sort();
    let allowed = ["project.db", "project.db-shm", "project.db-wal", "results"];
    ensure(
        names.iter().all(|name| {
            allowed.contains(&name.as_str())
                || (name.starts_with("findings-")
                    && [".db", ".db-shm", ".db-wal", ".lock"]
                        .iter()
                        .any(|suffix| name.ends_with(suffix)))
        }),
        "names.iter().all(|name| allowed.contains(&name.as_str()) || (name.starts_with(\"findings-\") && [\".db\", \".db-shm\", \".db-wal\", \".lock\"] .iter() .any(|suffix| name.ends_with(suffix))))",
    )?;
    ensure(
        names.contains(&"project.db".to_owned()),
        "names.contains(&\"project.db\".to_owned())",
    )?;
    ensure(
        names.iter().any(|name| {
            name.starts_with("findings-")
                && std::path::Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("db"))
        }),
        "names.iter().any(|name| name.starts_with(\"findings-\") && std::path::Path::new(name) .extension() .is_some_and(|extension| extension.eq_ignore_ascii_case(\"db\")))",
    )?;
    Ok(())
}

#[test]
fn the_mechanics_note_is_on_the_first_report_of_a_server_only() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let first = report_of(&manager, &path)?;
    let second = report_of(&manager, &path)?;
    let other_server = report_of(&sync::SourceSyncManager::default(), &path)?;

    let note = first
        .mechanics
        .note
        .check("the first report carries the note")?;
    ensure(
        note.contains("counting primitives, not a checklist"),
        "note.contains(\"counting primitives, not a checklist\")",
    )?;
    ensure(
        note.contains("Entries last until this server exits"),
        "note.contains(\"Entries last until this server exits\")",
    )?;
    ensure_eq(
        &second.mechanics.note,
        &None,
        "second.mechanics.note == None",
    )?;
    other_server
        .mechanics
        .note
        .as_ref()
        .check("a new server instance starts a new session")?;
    Ok(())
}

#[test]
fn leads_and_report_list_session_then_builtin() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();
    call(
        &manager,
        add(
            &path,
            "py_exec",
            "python",
            &EVAL_QUERY.replace("eval", "exec"),
        ),
    )
    .check("first add")?;
    call(&manager, add(&path, "py_eval", "python", EVAL_QUERY)).check("second add")?;

    let leads = run_leads(&manager, leads_request(&path, None, LeadMode::Full))?;
    let report = report_of(&manager, &path)?;

    let listed = sources(&leads);
    ensure_eq(
        listed.first().check("listed[0]")?,
        &("py_exec".to_owned(), MechanicSource::Session),
        "listed[0] == (\"py_exec\".to_owned(), MechanicSource::Session)",
    )?;
    ensure_eq(
        listed.get(1).check("listed[1]")?,
        &("py_eval".to_owned(), MechanicSource::Session),
        "listed[1] == (\"py_eval\".to_owned(), MechanicSource::Session)",
    )?;
    ensure(
        listed
            .get(2..)
            .check("listed[2..]")?
            .iter()
            .all(|(_, source)| *source == MechanicSource::Builtin),
        "listed[2..] .iter() .all(|(_, source)| *source == MechanicSource::Builtin)",
    )?;
    ensure_eq(
        &listed.len(),
        &(2 + crate::tools::leads_mechanics::MECHANICS.len()),
        "listed.len() == 2 + crate::tools::leads_mechanics::MECHANICS.len()",
    )?;
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
    ensure_eq(&reported, &expected, "reported == expected")?;
    Ok(())
}

#[test]
fn a_capitalised_language_is_refused_with_the_naming_convention() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "py_eval", "Python", EVAL_QUERY))?;

    assert_refused(&error, "lowercase pack name the index stores")?;
    Ok(())
}

#[test]
fn a_pack_alias_language_is_refused_with_its_canonical_name() -> TestResult {
    let project = python_project()?;
    let path = path_of(&project)?;
    let manager = sync::SourceSyncManager::default();

    let error = refusal(&manager, add(&path, "sh_eval", "shell", "(command) @c"))?;

    assert_refused(&error, "use `bash`")?;
    Ok(())
}

#[path = "mechanic_refusal_tests.rs"]
mod refusal_tests;
