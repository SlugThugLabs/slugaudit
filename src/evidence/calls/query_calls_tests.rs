//! Call evidence for every language with an embedded call query. Each
//! fixture holds decoys — a call-shaped text in a comment and in a string —
//! that must not count, one plain call, one member call, and one nested call
//! (`outer(inner(x))` counts two). Grammars download on first use; a missing
//! one fails the test with a clear message, never a vacuous pass.

use super::extract;
use crate::evidence::calls::call_queries::CALL_QUERIES;
use crate::model::{EvidenceItem, EvidenceKind, LineIndex};
use slugaudit_test_check::{Check, TestResult};

/// Fails, naming the language, when its grammar cannot be loaded — through
/// the same `get_language` the extraction uses.
fn require_grammar(language: &str) -> TestResult<tree_sitter::Language> {
    tree_sitter_language_pack::get_language(language)
        .map_err(|error| {
            format!(
                "grammar for {language} is not available on this host; run once with network access ({error})"
            )
        })
        .check("load the grammar")
}

/// The `Call` rows the full normalization produces for `source`, so the
/// dispatch from `normalize::extract_within` is exercised too.
fn calls(language: &str, source: &str) -> TestResult<Vec<EvidenceItem>> {
    require_grammar(language)?;
    let (items, truncated) =
        crate::evidence::extract(language, source).check("extract the fixture")?;
    assert!(!truncated, "{language} fixture extraction was cut short");
    Ok(items
        .into_iter()
        .filter(|item| item.kind == EvidenceKind::Call)
        .collect())
}

/// `(callee_name, 1-based stored line, callee_text)` per row, in row order.
fn summary(rows: &[EvidenceItem]) -> TestResult<Vec<(String, i64, String)>> {
    rows.iter()
        .map(|row| {
            let stored = crate::evidence::to_row(row);
            Ok((
                row.payload["callee_name"]
                    .as_str()
                    .check("callee_name")?
                    .to_owned(),
                stored.start_line.check("a stored line")?,
                row.payload["callee_text"]
                    .as_str()
                    .check("callee_text")?
                    .to_owned(),
            ))
        })
        .collect()
}

/// Asserts the exact rows, and that every row names `language`, the
/// fixture's enclosing function as caller, and the query producer.
fn assert_calls(
    language: &str,
    source: &str,
    caller: &str,
    expected: &[(&str, i64, &str)],
) -> TestResult {
    let rows = calls(language, source)?;

    let expected: Vec<(String, i64, String)> = expected
        .iter()
        .map(|(name, line, text)| ((*name).to_owned(), *line, (*text).to_owned()))
        .collect();
    assert_eq!(summary(&rows)?, expected, "{language}: {rows:?}");
    for row in &rows {
        assert_eq!(row.payload["language"], language);
        assert_eq!(row.payload["caller"], caller, "{language}: {row:?}");
        assert_eq!(row.payload["resolution"], "unresolved-syntactic");
        assert_eq!(row.payload["confidence"], "medium");
        assert_eq!(crate::evidence::to_row(row).origin, "RawTree");
    }
    Ok(())
}

const PYTHON: &str = "# decoy: fake_call(1) in a comment
def run(child, x):
    text = \"string_call(2)\"
    print(x)
    child.expect(\"prompt\")
    outer(inner(x))
";

#[test]
fn python_calls_count_plain_attribute_and_nested_calls_but_not_decoys() -> TestResult {
    assert_calls(
        "python",
        PYTHON,
        "run",
        &[
            ("print", 4, "print"),
            ("expect", 5, "child.expect"),
            ("outer", 6, "outer"),
            ("inner", 6, "inner"),
        ],
    )?;
    Ok(())
}

const JAVASCRIPT: &str = "// decoy: fakeCall(1) in a comment
function run(child, x) {
  const text = \"stringCall(2)\";
  print(x);
  child.expect(\"prompt\");
  outer(inner(x));
}
";

#[test]
fn javascript_calls_count_plain_member_and_nested_calls_but_not_decoys() -> TestResult {
    assert_calls(
        "javascript",
        JAVASCRIPT,
        "run",
        &[
            ("print", 4, "print"),
            ("expect", 5, "child.expect"),
            ("outer", 6, "outer"),
            ("inner", 6, "inner"),
        ],
    )?;
    Ok(())
}

const TYPESCRIPT: &str = "// decoy: fakeCall(1) in a comment
function run(child: Child, x: number): void {
  const text: string = \"stringCall(2)\";
  print(x);
  child.expect(\"prompt\");
  outer(inner(x));
}
";

#[test]
fn typescript_calls_count_plain_member_and_nested_calls_but_not_decoys() -> TestResult {
    assert_calls(
        "typescript",
        TYPESCRIPT,
        "run",
        &[
            ("print", 4, "print"),
            ("expect", 5, "child.expect"),
            ("outer", 6, "outer"),
            ("inner", 6, "inner"),
        ],
    )?;
    Ok(())
}

const TSX: &str = "// decoy: fakeCall(1) in a comment
function Run({ child, x }: Props) {
  const text = \"stringCall(2)\";
  print(x);
  child.expect(\"prompt\");
  return <div title=\"jsxCall(3)\">{outer(inner(x))}</div>;
}
";

#[test]
fn tsx_calls_count_plain_member_and_nested_calls_but_not_decoys() -> TestResult {
    assert_calls(
        "tsx",
        TSX,
        "Run",
        &[
            ("print", 4, "print"),
            ("expect", 5, "child.expect"),
            ("outer", 6, "outer"),
            ("inner", 6, "inner"),
        ],
    )?;
    Ok(())
}

const GO: &str = "package main

// decoy: fakeCall(1) in a comment
func run(child Child, x int) {
\ttext := \"stringCall(2)\"
\tprint(x)
\tchild.Expect(\"prompt\")
\touter(inner(x))
}
";

#[test]
fn go_calls_count_plain_selector_and_nested_calls_but_not_decoys() -> TestResult {
    assert_calls(
        "go",
        GO,
        "run",
        &[
            ("print", 6, "print"),
            ("Expect", 7, "child.Expect"),
            ("outer", 8, "outer"),
            ("inner", 8, "inner"),
        ],
    )?;
    Ok(())
}

const JAVA: &str = "class Fixture {
  // decoy: fakeCall(1) in a comment
  void run(Child child, int x) {
    String text = \"stringCall(2)\";
    print(x);
    child.expect(\"prompt\");
    outer(inner(x));
  }
}
";

#[test]
fn java_calls_count_plain_qualified_and_nested_invocations_but_not_decoys() -> TestResult {
    assert_calls(
        "java",
        JAVA,
        "run",
        &[
            ("print", 5, "print"),
            ("expect", 6, "child.expect"),
            ("outer", 7, "outer"),
            ("inner", 7, "inner"),
        ],
    )?;
    Ok(())
}

const C: &str = "/* decoy: fake_call(1) in a comment */
void run(struct child *child, int x) {
  const char *text = \"string_call(2)\";
  print(x);
  child->expect(\"prompt\");
  outer(inner(x));
}
";

#[test]
fn c_calls_count_plain_field_and_nested_calls_but_not_decoys() -> TestResult {
    assert_calls(
        "c",
        C,
        "run",
        &[
            ("print", 4, "print"),
            ("expect", 5, "child->expect"),
            ("outer", 6, "outer"),
            ("inner", 6, "inner"),
        ],
    )?;
    Ok(())
}

#[test]
fn a_c_function_returning_a_pointer_is_still_named_as_the_caller() -> TestResult {
    let source = "char *name(void) { return lookup(1); }\n";

    let rows = calls("c", source)?;

    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].payload["caller"], "name");
    Ok(())
}

#[test]
fn a_call_at_file_scope_or_in_an_anonymous_function_has_no_caller() -> TestResult {
    let source = "setup()\nhandler = lambda: fire()\n";

    let rows = calls("python", source)?;

    assert_eq!(summary(&rows)?.len(), 2, "{rows:?}");
    assert!(
        rows.iter().all(|row| row.payload["caller"].is_null()),
        "{rows:?}"
    );
    Ok(())
}

#[test]
fn every_embedded_call_query_compiles_with_exactly_one_callee_capture() -> TestResult {
    for entry in &CALL_QUERIES {
        let grammar = require_grammar(entry.language)?;

        let query = entry
            .compiled(&grammar)
            .check("the embedded query compiles")?;

        assert_eq!(query.capture_names(), ["callee"], "{}", entry.language);
    }
    Ok(())
}

#[test]
fn every_call_and_caller_node_kind_exists_in_its_grammar() -> TestResult {
    for entry in &CALL_QUERIES {
        let grammar = require_grammar(entry.language)?;
        for kind in std::iter::once(entry.call_kind).chain(entry.function_kinds.iter().copied()) {
            let compiled = tree_sitter::Query::new(&grammar, &format!("({kind}) @node"));
            assert!(
                compiled.is_ok(),
                "{}: `{kind}`: {compiled:?}",
                entry.language
            );
        }
    }
    Ok(())
}

#[test]
fn a_language_without_a_call_query_yields_no_calls_and_stays_complete() -> TestResult {
    let source = "x.unwrap()\n";

    let extraction =
        extract("ruby", source, &LineIndex::new(source), None).check("extract ruby calls")?;

    assert!(extraction.items.is_empty());
    assert!(!extraction.truncated);
    assert!(!extraction.timed_out);
    Ok(())
}

#[test]
fn a_call_query_under_a_spent_deadline_is_timed_out_not_truncated() -> TestResult {
    require_grammar("python")?;
    let spent = crate::util::Deadline::after(std::time::Duration::ZERO);

    let extraction = extract("python", PYTHON, &LineIndex::new(PYTHON), Some(&spent))
        .check("extract python calls")?;

    assert!(extraction.items.is_empty());
    assert!(extraction.timed_out);
    assert!(!extraction.truncated);
    Ok(())
}

#[test]
fn call_languages_are_rust_and_every_embedded_query_language() {
    let languages: Vec<&str> = super::super::call_languages().collect();

    assert_eq!(
        languages,
        [
            "rust",
            "python",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "java",
            "c"
        ]
    );
}
