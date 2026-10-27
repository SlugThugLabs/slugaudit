//! Phase 9 golden responses. Each test runs real tool calls over a small
//! project and compares the serialized response with a fixture under
//! `tests/golden/phase9/`, after normalising volatile values (the revision
//! id, absolute paths) and masking the paging fields that Phase 9.3 changes
//! by design. With `GOLDEN_PHASE9_RECORD=1` in the environment (read only)
//! the same tests write the fixtures instead; see `tests/golden/phase9/README.md`.
use crate::progress::NoopProgressSink;
use crate::sync::SourceSyncManager;
use crate::tools::{
    FindingReadRequest, FindingRequest, LeadMode, LeadsRequest, MechanicAction, MechanicRequest,
    QueryRequest, StructureRequest, finding, finding_read, leads, mechanic, query, structure,
};
use rmcp::handler::server::wrapper::Parameters;
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const RECORD_ENV: &str = "GOLDEN_PHASE9_RECORD";
const REVISION: &str = "<REVISION_ID>";
const PROJECT: &str = "<PROJECT>";
const MASK: &str = "<PAGING>";
/// Paging fields masked at comparison, wherever they appear in a response.
const PAGING_KEYS: [&str; 4] = ["offset", "next_offset", "expected_revision", "cursor"];

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("phase9")
}

fn path_of(project: &tempfile::TempDir) -> String {
    project
        .path()
        .to_str()
        .expect("temp path is UTF-8")
        .to_owned()
}

/// Replaces the revision id and the absolute project path with fixed
/// placeholders, in place.
fn normalise(value: &mut Value, project: &str) {
    match value {
        Value::String(text) => {
            if text.contains(project) {
                *text = text.replace(project, PROJECT);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| normalise(item, project)),
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if key == "revision_id" && item.is_string() {
                    *item = Value::String(REVISION.to_owned());
                } else {
                    normalise(item, project);
                }
            }
        }
        _ => {}
    }
}

/// Masks every paging field so a comparison ignores them.
fn mask_paging(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(mask_paging),
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if PAGING_KEYS.contains(&key.as_str()) {
                    *item = Value::String(MASK.to_owned());
                } else {
                    mask_paging(item);
                }
            }
        }
        _ => {}
    }
}

/// Records (record mode) or compares one case. `request` is stored with the
/// response for readability; only `response` is compared.
fn check_case<R: Serialize, S: Serialize>(
    name: &str,
    project: &tempfile::TempDir,
    request: &R,
    response: &S,
) {
    let root = path_of(project);
    let mut request = serde_json::to_value(request).expect("request serializes");
    let mut response = serde_json::to_value(response).expect("response serializes");
    normalise(&mut request, &root);
    normalise(&mut response, &root);
    let file = fixture_dir().join(format!("{name}.json"));
    if std::env::var(RECORD_ENV).as_deref() == Ok("1") {
        let fixture = json!({ "case": name, "request": request, "response": response });
        let mut text = serde_json::to_string_pretty(&fixture).expect("fixture serializes");
        text.push('\n');
        fs::create_dir_all(fixture_dir()).expect("create fixture dir");
        fs::write(&file, text).expect("write fixture");
        return;
    }
    let text = fs::read_to_string(&file).unwrap_or_else(|error| {
        panic!(
            "fixture {} is missing ({error}); run generate.sh",
            file.display()
        )
    });
    let mut expected: Value = serde_json::from_str(&text).expect("fixture is JSON");
    let mut expected = expected["response"].take();
    mask_paging(&mut expected);
    mask_paging(&mut response);
    assert_eq!(response, expected, "golden case {name} drifted");
}

fn project_with(files: &[(&str, String)]) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .expect("activate project");
    for (relative, content) in files {
        fs::write(project.path().join(relative), content).expect("write fixture file");
    }
    project
}

/// Three Rust files with four `unwrap` calls each, one `expect` and a TODO.
fn rust_project() -> tempfile::TempDir {
    let files: Vec<(String, String)> = (0..3)
        .map(|file| {
            let body: String = (0..4)
                .map(|n| format!("pub fn f{n}(x: Option<u8>) -> u8 {{ x.unwrap() }}\n"))
                .collect();
            let extra = "// TODO: revisit\npub fn g(x: Option<u8>) -> u8 { println!(\"hi\"); x.expect(\"nope\") }\n";
            (format!("mod{file}.rs"), format!("{body}{extra}"))
        })
        .collect();
    let refs: Vec<(&str, String)> = files
        .iter()
        .map(|(name, body)| (name.as_str(), body.clone()))
        .collect();
    project_with(&refs)
}

fn leads_request(project: &tempfile::TempDir, mechanic: Option<&str>) -> LeadsRequest {
    LeadsRequest {
        path: path_of(project),
        mechanic: mechanic.map(str::to_owned),
        pattern: None,
        limit: None,
        offset: None,
        expected_revision: None,
        mode: LeadMode::Full,
        page_bytes: None,
    }
}

fn run_leads(request: &LeadsRequest, manager: &SourceSyncManager) -> Value {
    let response = leads(
        &Parameters(LeadsRequest {
            path: request.path.clone(),
            mechanic: request.mechanic.clone(),
            pattern: request.pattern.clone(),
            limit: request.limit,
            offset: request.offset,
            expected_revision: request.expected_revision.clone(),
            mode: request.mode,
            page_bytes: request.page_bytes,
        }),
        &NoopProgressSink,
        manager,
    )
    .expect("leads succeeds")
    .0;
    serde_json::to_value(response).expect("response serializes")
}

fn leads_case(
    name: &str,
    project: &tempfile::TempDir,
    request: &LeadsRequest,
    manager: &SourceSyncManager,
) -> Value {
    let response = run_leads(request, manager);
    let echoed = json!({
        "mechanic": request.mechanic, "limit": request.limit, "offset": request.offset,
        "compact": request.mode == LeadMode::Compact,
    });
    check_case(name, project, &echoed, &response);
    response
}

#[test]
fn golden_phase9_leads_counts_rows_pages_and_compact() {
    let project = rust_project();
    let manager = SourceSyncManager::default();

    leads_case(
        "leads_counts_only",
        &project,
        &leads_request(&project, None),
        &manager,
    );
    let mut first = leads_request(&project, Some("unwrap_calls"));
    first.limit = Some(5);
    let page_one = leads_case("leads_rows_page1", &project, &first, &manager);
    let mut second = leads_request(&project, Some("unwrap_calls"));
    second.limit = Some(5);
    second.offset = page_one["page"]["next_offset"].as_u64().map(|n| n as usize);
    second.expected_revision = page_one["page"]["revision_id"].as_str().map(str::to_owned);
    leads_case("leads_rows_page2", &project, &second, &manager);
    let mut compact = leads_request(&project, None);
    compact.mode = LeadMode::Compact;
    compact.limit = Some(8);
    leads_case("leads_compact", &project, &compact, &manager);
}

fn mechanic_request(project: &tempfile::TempDir, action: MechanicAction) -> MechanicRequest {
    MechanicRequest {
        path: path_of(project),
        action,
        id: None,
        index: None,
        language: None,
        query: None,
        note: None,
        persist: false,
        limit: None,
        page_bytes: None,
    }
}

fn add_println_mechanic(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
) -> (MechanicRequest, Value) {
    let mut add = mechanic_request(project, MechanicAction::Add);
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
    .expect("mechanic add succeeds")
    .0;
    (
        add,
        serde_json::to_value(response).expect("response serializes"),
    )
}

fn clone_mechanic(request: &MechanicRequest) -> MechanicRequest {
    MechanicRequest {
        path: request.path.clone(),
        action: request.action,
        id: request.id.clone(),
        index: request.index,
        language: request.language.clone(),
        query: request.query.clone(),
        note: request.note.clone(),
        persist: request.persist,
        limit: request.limit,
        page_bytes: request.page_bytes,
    }
}

#[test]
fn golden_phase9_leads_custom_mechanic_rows() {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    add_println_mechanic(&project, &manager);

    let mut request = leads_request(&project, Some("println_calls"));
    request.limit = Some(2);
    leads_case("leads_custom_mechanic", &project, &request, &manager);
}

#[test]
fn golden_phase9_mechanic_add_and_list() {
    let project = rust_project();
    let manager = SourceSyncManager::default();

    let (add, added) = add_println_mechanic(&project, &manager);
    check_case("mechanic_add", &project, &add.id, &added);
    let list = mechanic_request(&project, MechanicAction::List);
    let listed = mechanic(
        &Parameters(clone_mechanic(&list)),
        &NoopProgressSink,
        &manager,
    )
    .expect("mechanic list succeeds")
    .0;
    check_case("mechanic_list", &project, &"list", &listed);
}

fn run_query(
    project: &tempfile::TempDir,
    manager: &SourceSyncManager,
    sql: &str,
    offset: usize,
    page_bytes: Option<usize>,
    expected_revision: Option<String>,
) -> Value {
    let response = query(
        &Parameters(QueryRequest {
            path: path_of(project),
            sql: sql.to_owned(),
            offset,
            expected_revision,
            page_bytes,
        }),
        &NoopProgressSink,
        manager,
    )
    .expect("query succeeds")
    .0;
    serde_json::to_value(response).expect("response serializes")
}

fn query_case(name: &str, sql: &str, page_bytes: Option<usize>) {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    let response = run_query(&project, &manager, sql, 0, page_bytes, None);
    check_case(
        name,
        &project,
        &json!({ "sql": sql, "page_bytes": page_bytes }),
        &response,
    );
}

#[test]
fn golden_phase9_query_plain_rows() {
    query_case(
        "query_plain_rows",
        "SELECT path, language FROM files ORDER BY path",
        None,
    );
}

#[test]
fn golden_phase9_query_clipped_first_row() {
    query_case(
        "query_clipped_cells",
        "SELECT replace(hex(zeroblob(1500)), '00', 'ab') AS t, zeroblob(1500) AS b, 7 AS n",
        Some(600),
    );
}

#[test]
fn golden_phase9_query_positive_infinity() {
    query_case(
        "query_non_finite_inf",
        "SELECT 1e308*10 AS pos, -1e308*10 AS neg, 1.5 AS ok",
        None,
    );
}

#[test]
fn golden_phase9_query_nan_expressions() {
    query_case(
        "query_nan_expressions",
        "SELECT 1e308*10*0 AS mul, 1e308*10 - 1e308*10 AS sub, golden_nan() AS udf",
        None,
    );
}

#[test]
fn golden_phase9_query_text_with_nul() {
    query_case("query_text_with_nul", "SELECT 'a'||char(0)||'b' AS s", None);
}

#[test]
fn golden_phase9_query_blob() {
    query_case(
        "query_blob",
        "SELECT x'00ff' AS b, zeroblob(0) AS empty",
        None,
    );
}

#[test]
fn golden_phase9_query_numeric_looking_text() {
    query_case(
        "query_numeric_text",
        "SELECT '007' AS a, '1e3' AS b, '-0' AS c, ' 5' AS d",
        None,
    );
}

#[test]
fn golden_phase9_query_null_and_integers() {
    query_case(
        "query_null",
        "SELECT NULL AS n, 1 AS one, 0.5 AS half",
        None,
    );
    query_case(
        "query_i64_extremes",
        "SELECT 9223372036854775807 AS max_i, -9223372036854775808 AS min_i, 9007199254740993 AS past_2_53, 1e15 AS big_real",
        None,
    );
}

#[test]
fn golden_phase9_query_multi_page() {
    let project = rust_project();
    let manager = SourceSyncManager::default();
    let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<60) SELECT i, printf('%050d', i) AS pad FROM n";
    let one = run_query(&project, &manager, sql, 0, Some(1000), None);
    check_case(
        "query_multi_page1",
        &project,
        &json!({ "sql": sql, "page_bytes": 1000 }),
        &one,
    );
    let next = one["page"]["next_offset"]
        .as_u64()
        .expect("a second page exists");
    let revision = one["page"]["revision_id"].as_str().map(str::to_owned);
    let two = run_query(&project, &manager, sql, next as usize, Some(1000), revision);
    check_case(
        "query_multi_page2",
        &project,
        &json!({ "sql": sql, "page_bytes": 1000, "page": 2 }),
        &two,
    );
}

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

#[test]
fn golden_phase9_finding_read_list_and_offset() {
    let _guard = crate::util::lock_or_recover(&crate::util::SESSION_TEST_LOCK, "session_test_lock");
    let project = rust_project();
    let manager = SourceSyncManager::default();
    for (n, title) in ["first", "second", "third"].iter().enumerate() {
        finding(
            &Parameters(FindingRequest {
                path: path_of(&project),
                file: format!("mod{n}.rs"),
                line_start: 1,
                line_end: 2,
                severity: "low".to_owned(),
                category: "test".to_owned(),
                title: (*title).to_owned(),
                description: format!("{title} finding description"),
            }),
            &NoopProgressSink,
            &manager,
        )
        .expect("finding is stored");
    }
    for (name, offset) in [("finding_read_list", 0), ("finding_read_offset1", 1)] {
        let response = finding_read(
            &Parameters(FindingReadRequest {
                path: path_of(&project),
                file: None,
                offset,
                expected_revision: None,
            }),
            &NoopProgressSink,
            &manager,
        )
        .expect("finding_read succeeds")
        .0;
        check_case(name, &project, &json!({ "offset": offset }), &response);
    }
}
