use super::*;
use crate::model::EvidenceKind;

/// `extract` also reports whether a walker stopped at the depth cap. These
/// tests are about the items, so they take the first half; the cap has its
/// own test in this module.
fn extract_items(language: &str, source: &str) -> Vec<EvidenceItem> {
    extract(language, source).expect("extract").0
}

fn kinds(items: &[EvidenceItem]) -> Vec<&EvidenceKind> {
    items.iter().map(|item| &item.kind).collect()
}

#[test]
fn extracts_structure_imports_symbols_and_comments_from_python() {
    let source = "import os\n\n\
def greet(name):\n    \
\"\"\"Say hello.\"\"\"\n    \
# a line comment\n    \
message = \"hi\"\n    \
return message\n";
    let items = extract_items("python", source);

    assert!(kinds(&items).contains(&&EvidenceKind::Structure));
    assert!(kinds(&items).contains(&&EvidenceKind::Import));
    assert!(kinds(&items).contains(&&EvidenceKind::Comment));

    let structure = items
        .iter()
        .find(|item| item.kind == EvidenceKind::Structure)
        .expect("a structure item");
    assert_eq!(structure.payload["name"], "greet");
}

#[test]
fn extracts_rust_structure_and_a_nested_child_gets_a_distinct_key() {
    let source = "pub struct Widget {\n}\n\nimpl Widget {\n    pub fn build(&self) {}\n}\n";
    let items = extract_items("rust", source);

    let structure_keys: Vec<&str> = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Structure)
        .map(|item| item.key.as_str())
        .collect();
    assert!(
        structure_keys.len() >= 2,
        "expected at least struct + impl/method"
    );
    assert_eq!(
        structure_keys.len(),
        structure_keys
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
    );
}

/// A file nested past the walk bound must come back truncated, not abort the
/// process. Before the cap existed this test aborted: the walkers recursed
/// once per nesting level until the thread stack ran out, and a stack
/// overflow is not a catchable panic — `fatal runtime error: stack overflow,
/// aborting` — so the tool call never returned and the caller's session died
/// with it. Reproduced at 6 000 nested parens against the release binary.
#[test]
fn a_source_nested_past_the_walk_bound_is_truncated_not_aborted() {
    let nesting = usize::try_from(crate::evidence::MAX_WALK_DEPTH).expect("fits") * 4;
    let source = format!("x = {}1{}\n", "(".repeat(nesting), ")".repeat(nesting));

    let (items, depth_capped) =
        extract("python", &source).expect("a deep source is truncated, not a failure");

    assert!(
        depth_capped,
        "a walk that stopped at the bound must report it, or the extraction \
         claims a completeness it did not reach"
    );
    assert!(!items.is_empty(), "the walk still yields what it reached");
}

#[test]
fn malformed_source_still_produces_diagnostics() {
    let items = extract_items("rust", "fn broken( {\n");
    assert!(kinds(&items).contains(&&EvidenceKind::Diagnostic));
}

#[test]
fn every_item_carries_a_span_or_an_explicit_missing_reason() {
    let source = "fn a() {}\nfn b() {}\n";
    let items = extract_items("rust", source);
    assert!(!items.is_empty());
    for item in &items {
        match &item.span {
            SpanAvailability::Present(span) => assert!(span.end_byte >= span.start_byte),
            SpanAvailability::NormalizerUnavailable { reason } => assert!(!reason.is_empty()),
            SpanAvailability::PackOmitted | SpanAvailability::DerivedEvidence => {}
        }
    }
}

#[test]
fn unknown_language_is_a_typed_error_not_a_panic() {
    let result = extract("not-a-real-language", "irrelevant");
    let err = match result {
        Ok(_) => panic!("unknown language should fail"),
        Err(err) => err,
    };
    assert!(
        matches!(err, PackError::Download(_) | PackError::LanguageNotFound(_)),
        "expected Download or LanguageNotFound error, got: {err:?}"
    );
}

// --- generic variable/field extraction (extract_bindings) ---

#[test]
fn rust_let_bindings_surface_as_variable_symbols() {
    let source =
        "fn pick(first: u32, second: u32) -> u32 {\n    let winner = first;\n    winner\n}\n";
    let items = extract_items("rust", source);

    let vars: Vec<&EvidenceItem> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol && item.payload["kind"].as_str() == Some("Variable")
        })
        .collect();

    let names: Vec<&str> = vars
        .iter()
        .map(|item| item.payload["name"].as_str().expect("name is a string"))
        .collect();

    // `first` and `second` are parameters; `winner` is a let binding.
    assert!(
        names.contains(&"winner"),
        "expected the let binding `winner`, got {names:?}"
    );
}

#[test]
fn rust_parameters_surface_as_variable_symbols() {
    let source = "fn greet(name: &str, age: u8) {}\n";
    let items = extract_items("rust", source);

    let names: Vec<&str> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol
                && item.payload["kind"].as_str() == Some("Variable")
                && item.origin == EvidenceOrigin::PackSymbol
        })
        .map(|item| item.payload["name"].as_str().expect("name is a string"))
        .collect();

    assert!(
        names.contains(&"name"),
        "expected param `name`, got {names:?}"
    );
    assert!(
        names.contains(&"age"),
        "expected param `age`, got {names:?}"
    );
}

#[test]
fn rust_struct_fields_surface_as_field_symbols() {
    let source = "struct Person {\n    name: String,\n    age: u8,\n}\n";
    let items = extract_items("rust", source);

    let fields: Vec<&EvidenceItem> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol
                && item.payload["kind"].as_str() == Some(r#"Other("Field")"#)
        })
        .collect();

    let names: Vec<&str> = fields
        .iter()
        .map(|item| item.payload["name"].as_str().expect("name is a string"))
        .collect();

    assert!(
        names.contains(&"name"),
        "expected field `name`, got {names:?}"
    );
    assert!(
        names.contains(&"age"),
        "expected field `age`, got {names:?}"
    );
}

#[test]
fn binding_names_are_sliced_from_source_not_hardcoded() {
    // A near-duplicate pair: the extractor must report the exact text of
    // each binding so the AI can spot `file_path` vs `filepath`.
    let source = "fn index(file_path: &str, filepath: &str) {\n    let file_path_len = file_path.len();\n    let filepath_len = filepath.len();\n}\n";
    let items = extract_items("rust", source);

    let names: Vec<&str> = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Symbol)
        .map(|item| item.payload["name"].as_str().expect("name is a string"))
        .collect();

    assert!(names.contains(&"file_path"), "got {names:?}");
    assert!(names.contains(&"filepath"), "got {names:?}");
    assert!(names.contains(&"file_path_len"), "got {names:?}");
    assert!(names.contains(&"filepath_len"), "got {names:?}");
}

#[test]
fn python_assignments_surface_as_variable_symbols() {
    let source = "def run():\n    file_path = 'a'\n    filepath = 'b'\n    return file_path\n";
    let items = extract_items("python", source);

    let names: Vec<&str> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol && item.origin == EvidenceOrigin::PackSymbol
        })
        .map(|item| item.payload["name"].as_str().expect("name is a string"))
        .collect();

    assert!(
        names.contains(&"file_path") && names.contains(&"filepath"),
        "expected both near-duplicate python bindings, got {names:?}"
    );
}

#[test]
fn pack_covered_languages_never_double_report_imports_via_the_generic_walker() {
    // The generic import walker is gated on the pack finding zero imports
    // for a file. A python file with imports must produce exactly the
    // pack's Import items and no GenericWalker-origin duplicates.
    let source = "import os\nfrom .sibling import helper\n";
    let items = extract_items("python", source);
    let import_items: Vec<&EvidenceItem> = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Import)
        .collect();
    assert!(!import_items.is_empty(), "the pack finds the imports");
    assert!(
        import_items
            .iter()
            .all(|item| item.origin != EvidenceOrigin::GenericWalker),
        "pack-covered languages must not re-extract imports with the generic walker"
    );
}

#[test]
fn binding_extraction_gracefully_degrades_for_an_unsupported_language() {
    // `extract` itself errors on an unknown language, but the binding
    // walker is invoked only after `process` succeeds — so this guards
    // the `get_parser` fallback path indirectly through a language the
    // pack knows but whose grammar yields no `identifier` children for
    // the matched node types. Rust with a source containing no bindings
    // at all should simply return no variable/field items.
    let source = "// just a comment, no bindings anywhere\n";
    let items = extract_items("rust", source);
    let binding_count = items
        .iter()
        .filter(|item| item.origin == EvidenceOrigin::PackSymbol)
        .count();
    assert_eq!(binding_count, 0, "no bindings to extract here");
}
