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
        .map(|item| {
            item.payload["callee_name"]
                .as_str()
                .expect("callee_name as string")
        })
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
fn method_calls_report_the_field_name_as_callee_name() {
    let source = "fn run(v: Option<u8>) -> u8 {\n    v.unwrap()\n}\n";
    let calls = extract_rust(source);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].payload["callee_name"], "unwrap");
    assert_eq!(calls[0].payload["callee_text"], "v.unwrap");
    assert_eq!(calls[0].payload["caller"], "run");
}

#[test]
fn malformed_rust_does_not_panic() {
    let calls = extract_rust("fn run() { helper(\n");
    assert!(calls.len() <= 1);
}

#[test]
fn extracts_method_calls_from_macro_arguments_without_duplicates() {
    let source = r#"
fn run(a: Option<u8>, b: Option<u8>) {
    assert_eq!(a.unwrap(), b.unwrap());
    assert!(Some(1).unwrap() == 1);
    debug_assert!(Some(2).unwrap() > 0);
    println!("{}", Some(3).unwrap());
    let _ = vec![Some(4).unwrap(), Some(5).unwrap()];
}
"#;
    let calls = extract_rust(source);
    let unwraps: Vec<_> = calls
        .iter()
        .filter(|item| item.payload["callee_name"] == "unwrap")
        .collect();
    assert_eq!(unwraps.len(), 6, "{calls:?}");
    for call in unwraps {
        assert_eq!(call.payload["caller"], "run");
        assert!(matches!(
            call.span,
            crate::model::SpanAvailability::Present(_)
        ));
    }
}

#[test]
fn macro_arguments_do_not_index_strings_or_comments_as_calls() {
    let source = r#"
fn run(value: Option<u8>) {
    println!("fake.unwrap() {}", value.unwrap()); // ignored.unwrap()
    assert!(value.is_some(), "text.unwrap()");
}
"#;
    let calls = extract_rust(source);
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.payload["callee_name"] == "unwrap")
            .count(),
        1,
        "{calls:?}"
    );
}

#[test]
fn nested_macros_do_not_duplicate_inner_method_calls() {
    let source = "fn run(v: Option<u8>) { assert!(matches!(v.unwrap(), 1)); }";
    let calls = extract_rust(source);
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.payload["callee_name"] == "unwrap")
            .count(),
        1,
        "{calls:?}"
    );
}
