//! `leads` over languages other than Rust: call-site mechanics count the
//! calls of every language with call extraction, and `covers` lists exactly
//! the languages each count read.
use super::*;
use crate::tools::query::{QueryRequest, query};

/// A Python file with one `expect` call (pexpect's idiom) on line 5, next
/// to decoys in a comment and a string that must not count, and three other
/// calls (`print`, `outer`, `inner`).
const PYTHON_FIXTURE: &str = "# decoy: child.expect(\"comment\")
def run(child, x):
    text = \"child.expect('string')\"
    print(x)
    child.expect(\"prompt\")
    outer(inner(x))
";

fn python_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(project.path().join("app.py"), PYTHON_FIXTURE).expect("write python source");
    project
}

#[test]
fn leads_counts_python_calls_and_covers_lists_python() {
    let project = python_project();

    let response = run(request(project.path(), Some("expect_calls")));

    let expect = entry(&response, "expect_calls");
    assert_eq!(
        expect.count, 1,
        "the comment and string decoys do not count"
    );
    assert_eq!(expect.covers, ["python"]);
    assert_eq!(entry(&response, "unwrap_calls").covers, ["python"]);
    assert_eq!(count_of(&response, "unwrap_calls"), 0);
    let rows = full_rows(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].file, "app.py");
    assert_eq!(rows[0].line, 5);
    assert_eq!(rows[0].detail, "child.expect");
}

#[test]
fn query_reads_every_python_call_as_call_evidence() {
    let project = python_project();
    let sql = "SELECT json_extract(e.payload, '$.callee_name') AS name, e.start_line AS line, \
               json_extract(e.payload, '$.language') AS language \
               FROM evidence e WHERE e.kind = 'Call' ORDER BY e.start_byte";

    let rows = query(
        &Parameters(QueryRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            sql: sql.to_owned(),
            cursor: None,
            legacy_paging: Default::default(),
            page_bytes: None,
        }),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .expect("query succeeds")
    .0
    .rows;

    let calls: Vec<(String, i64, String)> = rows
        .iter()
        .map(|row| {
            (
                row["name"].as_str().expect("name").to_owned(),
                row["line"].as_i64().expect("line"),
                row["language"].as_str().expect("language").to_owned(),
            )
        })
        .collect();
    let python = |name: &str, line| (name.to_owned(), line, "python".to_owned());
    assert_eq!(
        calls,
        [
            python("print", 4),
            python("expect", 5),
            python("outer", 6),
            python("inner", 6)
        ]
    );
}

#[test]
fn call_mechanics_cover_no_language_for_a_project_without_call_extraction() {
    let project = tempfile::tempdir().expect("project dir");
    activate(project.path());
    fs::write(
        project.path().join("app.rb"),
        b"def a(x)\n  x.unwrap()\nend\n",
    )
    .expect("write ruby source");

    let response = run(request(project.path(), None));

    // Ruby has no call query: the mechanic exists but measured nothing here,
    // so it reports zero rather than disappearing — and an empty `covers`
    // says the zero is unchecked, while comment markers did read Ruby.
    assert_eq!(count_of(&response, "unwrap_calls"), 0);
    assert!(entry(&response, "unwrap_calls").covers.is_empty());
    assert_eq!(entry(&response, "comment_markers").covers, ["ruby"]);
}

#[test]
fn covers_follows_the_pattern_and_keeps_both_call_languages_sorted() {
    let project = python_project();
    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )
    .expect("write rust source");

    let everything = run(request(project.path(), None));
    let mut rust_only = request(project.path(), None);
    rust_only.pattern = Some("*.rs".to_owned());
    let rust_only = run(rust_only);

    assert_eq!(
        entry(&everything, "unwrap_calls").covers,
        ["python", "rust"]
    );
    assert_eq!(entry(&rust_only, "unwrap_calls").covers, ["rust"]);
}
