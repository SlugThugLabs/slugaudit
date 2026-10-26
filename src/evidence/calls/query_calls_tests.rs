//! Call evidence for every language with an embedded call query. Each
//! fixture holds decoys — a call-shaped text in a comment and in a string —
//! that must not count, one plain call, one member call, and one nested call
//! (`outer(inner(x))` counts two). Grammars download on first use; a missing
//! one fails the test with a clear message, never a vacuous pass.

use super::extract;
use crate::evidence::calls::call_queries::CALL_QUERIES;
use crate::model::{EvidenceItem, EvidenceKind, LineIndex};

/// Fails, naming the language, when its grammar cannot be loaded — through
/// the same `get_language` the extraction uses.
fn require_grammar(language: &str) -> tree_sitter::Language {
    match tree_sitter_language_pack::get_language(language) {
        Ok(grammar) => grammar,
        Err(error) => panic!(
            "grammar for {language} is not available on this host; run once with network access ({error})"
        ),
    }
}

/// The `Call` rows the full normalization produces for `source`, so the
/// dispatch from `normalize::extract_within` is exercised too.
fn calls(language: &str, source: &str) -> Vec<EvidenceItem> {
    require_grammar(language);
    let (items, truncated) = crate::evidence::extract(language, source).expect("extraction");
    assert!(!truncated, "{language} fixture extraction was cut short");
    items
        .into_iter()
        .filter(|item| item.kind == EvidenceKind::Call)
        .collect()
}

/// `(callee_name, 1-based stored line, callee_text)` per row, in row order.
fn summary(rows: &[EvidenceItem]) -> Vec<(String, i64, String)> {
    rows.iter()
        .map(|row| {
            let stored = crate::evidence::to_row(row);
            (
                row.payload["callee_name"]
                    .as_str()
                    .expect("callee_name")
                    .to_owned(),
                stored.start_line.expect("a stored line"),
                row.payload["callee_text"]
                    .as_str()
                    .expect("callee_text")
                    .to_owned(),
            )
        })
        .collect()
}

/// Asserts the exact rows, and that every row names `language`, the
/// fixture's enclosing function as caller, and the query producer.
fn assert_calls(language: &str, source: &str, caller: &str, expected: &[(&str, i64, &str)]) {
    let rows = calls(language, source);

    let expected: Vec<(String, i64, String)> = expected
        .iter()
        .map(|(name, line, text)| ((*name).to_owned(), *line, (*text).to_owned()))
        .collect();
    assert_eq!(summary(&rows), expected, "{language}: {rows:?}");
    for row in &rows {
        assert_eq!(row.payload["language"], language);
        assert_eq!(row.payload["caller"], caller, "{language}: {row:?}");
        assert_eq!(row.payload["resolution"], "unresolved-syntactic");
        assert_eq!(row.payload["confidence"], "medium");
        assert_eq!(crate::evidence::to_row(row).origin, "RawTree");
    }
}

const PYTHON: &str = "# decoy: fake_call(1) in a comment
def run(child, x):
    text = \"string_call(2)\"
    print(x)
    child.expect(\"prompt\")
    outer(inner(x))
";

#[test]
fn python_calls_count_plain_attribute_and_nested_calls_but_not_decoys() {
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
    );
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
fn javascript_calls_count_plain_member_and_nested_calls_but_not_decoys() {
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
    );
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
fn typescript_calls_count_plain_member_and_nested_calls_but_not_decoys() {
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
    );
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
fn tsx_calls_count_plain_member_and_nested_calls_but_not_decoys() {
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
    );
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
fn go_calls_count_plain_selector_and_nested_calls_but_not_decoys() {
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
    );
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
fn java_calls_count_plain_qualified_and_nested_invocations_but_not_decoys() {
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
    );
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
fn c_calls_count_plain_field_and_nested_calls_but_not_decoys() {
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
    );
}

#[test]
fn a_c_function_returning_a_pointer_is_still_named_as_the_caller() {
    let source = "char *name(void) { return lookup(1); }\n";

    let rows = calls("c", source);

    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].payload["caller"], "name");
}

#[test]
fn a_call_at_file_scope_or_in_an_anonymous_function_has_no_caller() {
    let source = "setup()\nhandler = lambda: fire()\n";

    let rows = calls("python", source);

    assert_eq!(summary(&rows).len(), 2, "{rows:?}");
    assert!(
        rows.iter().all(|row| row.payload["caller"].is_null()),
        "{rows:?}"
    );
}

#[test]
fn every_embedded_call_query_compiles_with_exactly_one_callee_capture() {
    for entry in &CALL_QUERIES {
        let grammar = require_grammar(entry.language);

        let query = entry
            .compiled(&grammar)
            .expect("the embedded query compiles");

        assert_eq!(query.capture_names(), ["callee"], "{}", entry.language);
    }
}

#[test]
fn every_call_and_caller_node_kind_exists_in_its_grammar() {
    for entry in &CALL_QUERIES {
        let grammar = require_grammar(entry.language);
        for kind in std::iter::once(entry.call_kind).chain(entry.function_kinds.iter().copied()) {
            let compiled = tree_sitter::Query::new(&grammar, &format!("({kind}) @node"));
            assert!(
                compiled.is_ok(),
                "{}: `{kind}`: {compiled:?}",
                entry.language
            );
        }
    }
}

#[test]
fn a_language_without_a_call_query_yields_no_calls_and_stays_complete() {
    let source = "x.unwrap()\n";

    let extraction = extract("ruby", source, &LineIndex::new(source), None);

    assert!(extraction.items.is_empty());
    assert!(!extraction.truncated);
    assert!(!extraction.timed_out);
}

#[test]
fn a_call_query_under_a_spent_deadline_is_timed_out_not_truncated() {
    require_grammar("python");
    let spent = crate::util::Deadline::after(std::time::Duration::ZERO);

    let extraction = extract("python", PYTHON, &LineIndex::new(PYTHON), Some(&spent));

    assert!(extraction.items.is_empty());
    assert!(extraction.timed_out);
    assert!(!extraction.truncated);
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
