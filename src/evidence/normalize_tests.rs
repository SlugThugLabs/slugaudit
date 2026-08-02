use super::*;
use crate::model::EvidenceKind;

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
    let items = extract("python", source).expect("process python");

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
    let items = extract("rust", source).expect("process rust");

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

#[test]
fn malformed_source_still_produces_diagnostics() {
    let items = extract("rust", "fn broken( {\n").expect("process even with syntax errors");
    assert!(kinds(&items).contains(&&EvidenceKind::Diagnostic));
}

#[test]
fn every_item_carries_a_span_or_an_explicit_missing_reason() {
    let source = "fn a() {}\nfn b() {}\n";
    let items = extract("rust", source).expect("process rust");
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
    assert!(result.is_err());
}
