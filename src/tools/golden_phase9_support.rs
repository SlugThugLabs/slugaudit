//! Shared support for the Phase 9 golden tests. Each test runs real tool calls over a small
//! project and compares the serialized response with a fixture under
//! `tests/golden/phase9/`, after normalising volatile values (the revision
//! id, absolute paths) and masking the paging fields that differ
//! by design. With `GOLDEN_PHASE9_RECORD=1` in the environment (read only)
//! the same tests write the fixtures instead; see `tests/golden/phase9/README.md`.
use serde::Serialize;
use serde_json::{Value, json};
use slugaudit_test_check::{Check, TestResult};
use std::fs;
use std::path::{Path, PathBuf};

const RECORD_ENV: &str = "GOLDEN_PHASE9_RECORD";
const REVISION: &str = "<REVISION_ID>";
const PROJECT: &str = "<PROJECT>";
/// Paging fields removed at comparison (see [`mask_paging`] for where), so a
/// response with or without them compares equal.
const PAGING_KEYS: [&str; 6] = [
    "offset",
    "next_offset",
    "expected_revision",
    "cursor",
    "completeness",
    // Not a `query` response field; the fixtures recorded it as null.
    "total_unavailable",
];

pub(super) fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("phase9")
}

pub(super) fn path_of(project: &tempfile::TempDir) -> TestResult<String> {
    Ok(project
        .path()
        .to_str()
        .check("temp path is UTF-8")?
        .to_owned())
}

/// Replaces the revision id and the absolute project path with fixed
/// placeholders, in place.
pub(super) fn normalise(value: &mut Value, project: &str) {
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

/// Removes the paging fields so a comparison ignores them: at the top level
/// of the response and inside every `page` object, found by walking nested
/// objects only. Arrays are never entered, so a result row whose column is
/// aliased `cursor` or `offset` keeps its data.
pub(super) fn mask_paging(value: &mut Value) {
    mask_object(value, true);
}

fn mask_object(value: &mut Value, paging_scope: bool) {
    let Value::Object(map) = value else {
        return;
    };
    if paging_scope {
        map.retain(|key, _| !PAGING_KEYS.contains(&key.as_str()));
    }
    for (key, item) in map.iter_mut() {
        mask_object(item, key == "page");
    }
}

/// Records (record mode) or compares one case. `request` is stored with the
/// response for readability; only `response` is compared.
pub(super) fn check_case<R: Serialize, S: Serialize>(
    name: &str,
    project: &tempfile::TempDir,
    request: &R,
    response: &S,
) -> TestResult {
    let root = path_of(project)?;
    let mut request = serde_json::to_value(request).check("request serializes")?;
    let mut response = serde_json::to_value(response).check("response serializes")?;
    normalise(&mut request, &root);
    normalise(&mut response, &root);
    let file = fixture_dir().join(format!("{name}.json"));
    if std::env::var(RECORD_ENV).as_deref() == Ok("1") {
        let fixture = json!({ "case": name, "request": request, "response": response });
        let mut text = serde_json::to_string_pretty(&fixture).check("fixture serializes")?;
        text.push('\n');
        fs::create_dir_all(fixture_dir()).check("create fixture dir")?;
        fs::write(&file, text).check("write fixture")?;
        return Ok(());
    }
    let text = fs::read_to_string(&file).check(&format!(
        "fixture {} is missing; run generate.sh",
        file.display()
    ))?;
    let mut fixture: Value = serde_json::from_str(&text).check("fixture is JSON")?;
    let mut expected = fixture
        .get_mut("response")
        .check("fixture has a `response`")?
        .take();
    mask_paging(&mut expected);
    mask_paging(&mut response);
    assert_eq!(response, expected, "golden case {name} drifted");
    Ok(())
}

pub(super) fn project_with(files: &[(&str, String)]) -> TestResult<tempfile::TempDir> {
    let project = tempfile::tempdir().check("project dir")?;
    fs::create_dir_all(project.path().join(".planning").join("slugaudit"))
        .check("activate project")?;
    crate::project::write_database_marker(&project.path().join(".planning").join("slugaudit"))?;
    for (relative, content) in files {
        fs::write(project.path().join(relative), content).check("write fixture file")?;
    }
    Ok(project)
}

/// Three Rust files with four `unwrap` calls each, one `expect` and a TODO.
pub(super) fn rust_project() -> TestResult<tempfile::TempDir> {
    let files: Vec<(String, String)> = (0..3)
        .map(|file| {
            let body = (0..4)
                .map(|n| format!("pub fn f{n}(x: Option<u8>) -> u8 {{ x.unwrap() }}\n"))
                .collect::<Vec<_>>()
                .concat();
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
