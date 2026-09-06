use super::extract_rust;
use crate::model::EvidenceKind;

#[test]
fn extracts_direct_qualified_and_macro_calls() {
    let source = "fn run() {\n    helper();\n    crate::worker::start();\n    println!(\"ok\");\n}\nfn helper() {}\n";
    let calls = extract_rust(source);
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|item| item.kind == EvidenceKind::Call));

    let names: Vec<&str> = calls
        .iter()
        .map(|item| item.payload["callee_name"].as_str().expect("callee_name as string"))
        .collect();
    assert!(names.contains(&"helper"));
    assert!(names.contains(&"start"));
    assert!(names.contains(&"println"));

    let run_call = calls
        .iter()
        .find(|item| item.payload["callee_name"] == "helper")
        .expect("expected run_call helper");
    assert_eq!(run_call.payload["caller"], "run");
    assert_eq!(run_call.payload["resolution"], "unresolved-syntactic");

    let macro_call = calls
        .iter()
        .find(|item| item.payload["callee_name"] == "println")
        .expect("expected macro_call println");
    assert_eq!(macro_call.payload["resolution"], "macro");
}

#[test]
fn calls_have_spans_and_do_not_include_definition_names() {
    let source = "fn build() { make(); }\nfn make() {}\n";
    let calls = extract_rust(source);
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.payload["callee_text"], "make");
    assert_eq!(call.payload["caller"], "build");
    assert!(matches!(
        call.span,
        crate::model::SpanAvailability::Present(_)
    ));
}

#[test]
fn malformed_rust_does_not_panic() {
    let calls = extract_rust("fn run() { helper(\n");
    assert!(calls.len() <= 1);
}
