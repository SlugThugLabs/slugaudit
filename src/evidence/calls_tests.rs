use crate::evidence::{Extraction, WalkInput};
use crate::model::LineIndex;
use crate::util::Deadline;
use slugaudit_test_check::{Check, TestResult};

/// Parses `source` (unbounded) and runs the Rust call walker over it within
/// `deadline`.
fn walk_rust(source: &str, deadline: Option<&Deadline>) -> TestResult<Extraction> {
    let tree = crate::evidence::parse_tree("rust", source, None)
        .map_err(|failure| format!("{failure:?}"))
        .check("parse the Rust source")?;
    let lines = LineIndex::new(source);
    let input = WalkInput {
        source,
        lines: &lines,
        deadline,
    };
    super::extract_rust(&tree, &input).check("extract Rust calls")
}

fn extract_rust(source: &str) -> TestResult<Vec<crate::model::EvidenceItem>> {
    Ok(walk_rust(source, None)?.items)
}

/// The `caller` of every call named `callee`, in source order.
fn callers_of<'a>(
    calls: &'a [crate::model::EvidenceItem],
    callee: &str,
) -> Vec<&'a serde_json::Value> {
    calls
        .iter()
        .filter(|item| item.payload["callee_name"] == callee)
        .map(|item| &item.payload["caller"])
        .collect()
}
use crate::model::EvidenceKind;

#[test]
fn extracts_direct_qualified_and_macro_calls() -> TestResult {
    let source = "fn run() {\n    helper();\n    crate::worker::start();\n    println!(\"ok\");\n}\nfn helper() {}\n";
    let calls = extract_rust(source)?;
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|item| item.kind == EvidenceKind::Call));

    let names: Vec<&str> = calls
        .iter()
        .map(|item| {
            item.payload["callee_name"]
                .as_str()
                .check("callee_name as string")
        })
        .collect::<TestResult<_>>()?;
    assert!(names.contains(&"helper"));
    assert!(names.contains(&"start"));
    assert!(names.contains(&"println"));

    let run_call = calls
        .iter()
        .find(|item| item.payload["callee_name"] == "helper")
        .check("expected run_call helper")?;
    assert_eq!(run_call.payload["caller"], "run");
    assert_eq!(run_call.payload["resolution"], "unresolved-syntactic");

    let macro_call = calls
        .iter()
        .find(|item| item.payload["callee_name"] == "println")
        .check("expected macro_call println")?;
    assert_eq!(macro_call.payload["resolution"], "macro");
    Ok(())
}

#[test]
fn calls_have_spans_and_do_not_include_definition_names() -> TestResult {
    let source = "fn build() { make(); }\nfn make() {}\n";
    let calls = extract_rust(source)?;
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.payload["callee_text"], "make");
    assert_eq!(call.payload["caller"], "build");
    assert!(matches!(
        call.span,
        crate::model::SpanAvailability::Present(_)
    ));
    Ok(())
}

#[test]
fn method_calls_report_the_field_name_as_callee_name() -> TestResult {
    let source = "fn run(v: Option<u8>) -> u8 {\n    v.unwrap()\n}\n";
    let calls = extract_rust(source)?;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].payload["callee_name"], "unwrap");
    assert_eq!(calls[0].payload["callee_text"], "v.unwrap");
    assert_eq!(calls[0].payload["caller"], "run");
    Ok(())
}

#[test]
fn turbofish_callees_report_the_inner_function_or_method_name() -> TestResult {
    let source = "fn run(x: &str) {\n    foo::<u8>();\n    x.parse::<u32>();\n}\n";

    let calls = extract_rust(source)?;

    let named: Vec<(&str, &str, &str)> = calls
        .iter()
        .map(|item| {
            Ok((
                item.payload["callee_name"].as_str().check("callee_name")?,
                item.payload["callee_text"].as_str().check("callee_text")?,
                item.payload["resolution"].as_str().check("resolution")?,
            ))
        })
        .collect::<TestResult<_>>()?;
    assert_eq!(
        named,
        [
            ("foo", "foo::<u8>", "unresolved-syntactic"),
            ("parse", "x.parse::<u32>", "unresolved-syntactic"),
        ]
    );
    Ok(())
}

#[test]
fn a_qualified_turbofish_callee_stays_qualified() -> TestResult {
    let source = "fn run() { std::mem::size_of::<u8>(); }\n";

    let calls = extract_rust(source)?;

    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].payload["callee_name"], "size_of");
    assert_eq!(calls[0].payload["resolution"], "qualified-syntactic");
    assert_eq!(calls[0].payload["confidence"], "high");
    Ok(())
}

#[test]
fn a_qualified_call_inside_a_macro_has_the_same_confidence_as_outside() -> TestResult {
    let source = "fn run() { println!(\"{}\", crate::util::now()); }\n";

    let calls = extract_rust(source)?;

    let now = calls
        .iter()
        .find(|item| item.payload["callee_name"] == "now")
        .check("the macro argument call is recovered")?;
    assert_eq!(now.payload["resolution"], "qualified-syntactic");
    assert_eq!(now.payload["confidence"], "high");
    Ok(())
}

#[test]
fn malformed_rust_does_not_panic() -> TestResult {
    let calls = extract_rust("fn run() { helper(\n")?;
    assert!(calls.len() <= 1);
    Ok(())
}

#[test]
fn extracts_method_calls_from_macro_arguments_without_duplicates() -> TestResult {
    let source = r#"
fn run(a: Option<u8>, b: Option<u8>) {
    assert_eq!(a.unwrap(), b.unwrap());
    assert!(Some(1).unwrap() == 1);
    debug_assert!(Some(2).unwrap() > 0);
    println!("{}", Some(3).unwrap());
    let _ = vec![Some(4).unwrap(), Some(5).unwrap()];
}
"#;
    let calls = extract_rust(source)?;
    let unwraps: Vec<_> = calls
        .iter()
        .filter(|item| item.payload["callee_name"] == "unwrap")
        .collect();
    assert_eq!(unwraps.len(), 7, "{calls:?}");
    for call in unwraps {
        assert_eq!(call.payload["caller"], "run");
        assert!(matches!(
            call.span,
            crate::model::SpanAvailability::Present(_)
        ));
    }
    Ok(())
}

#[test]
fn macro_arguments_do_not_index_strings_or_comments_as_calls() -> TestResult {
    let source = r#"
fn run(value: Option<u8>) {
    println!("fake.unwrap() {}", value.unwrap()); // ignored.unwrap()
    assert!(value.is_some(), "text.unwrap()");
}
"#;
    let calls = extract_rust(source)?;
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.payload["callee_name"] == "unwrap")
            .count(),
        1,
        "{calls:?}"
    );
    Ok(())
}

#[test]
fn nested_macros_do_not_duplicate_inner_method_calls() -> TestResult {
    let source = "fn run(v: Option<u8>) { assert!(matches!(v.unwrap(), 1)); }";
    let calls = extract_rust(source)?;
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.payload["callee_name"] == "unwrap")
            .count(),
        1,
        "{calls:?}"
    );
    Ok(())
}

#[test]
fn a_call_walk_under_a_spent_deadline_is_timed_out_not_truncated() -> TestResult {
    let source = "fn run() { helper(); }\n";
    let spent = Deadline::after(std::time::Duration::ZERO);

    let extraction = walk_rust(source, Some(&spent))?;

    assert!(extraction.items.is_empty());
    assert!(extraction.timed_out);
    assert!(!extraction.truncated);
    Ok(())
}

/// A call inside a closure reports no caller, inside a macro argument as
/// outside one: the closure, not the function around it, encloses the call.
#[test]
fn a_call_in_a_closure_has_no_caller_inside_and_outside_a_macro() -> TestResult {
    let source = "fn run(v: Vec<u8>) {\n    \
                  v.iter().for_each(|x| plain(x));\n    \
                  println!(\"{:?}\", v.iter().map(|x| wrapped(x)));\n    \
                  direct();\n    \
                  println!(\"{}\", in_macro());\n}\n";

    let calls = extract_rust(source)?;

    let null = serde_json::Value::Null;
    let run = serde_json::Value::from("run");
    assert_eq!(callers_of(&calls, "plain"), [&null], "{calls:?}");
    assert_eq!(callers_of(&calls, "wrapped"), [&null], "{calls:?}");
    assert_eq!(callers_of(&calls, "direct"), [&run], "{calls:?}");
    assert_eq!(callers_of(&calls, "in_macro"), [&run], "{calls:?}");
    assert_eq!(callers_of(&calls, "for_each"), [&run], "{calls:?}");
    assert_eq!(callers_of(&calls, "map"), [&run], "{calls:?}");
    Ok(())
}

/// A call reports the innermost named function around it, and a call after
/// a nested function reports the outer one again.
#[test]
fn a_call_reports_the_innermost_enclosing_function() -> TestResult {
    let source = "fn outer() {\n    \
                  fn inner() { deep(); }\n    \
                  shallow();\n}\n\
                  impl T { fn method(&self) { in_method(); } }\n\
                  const C: u8 = top();\n";

    let calls = extract_rust(source)?;

    assert_eq!(
        callers_of(&calls, "deep"),
        [&serde_json::Value::from("inner")]
    );
    assert_eq!(
        callers_of(&calls, "shallow"),
        [&serde_json::Value::from("outer")]
    );
    assert_eq!(
        callers_of(&calls, "in_method"),
        [&serde_json::Value::from("method")]
    );
    assert_eq!(callers_of(&calls, "top"), [&serde_json::Value::Null]);
    Ok(())
}

#[test]
fn a_macro_body_re_parse_under_a_spent_deadline_is_timed_out() -> TestResult {
    let source = "fn run() { println!(\"{}\", helper()); }\n";
    let lines = LineIndex::new(source);
    let tree = crate::evidence::parse_tree("rust", source, None)
        .map_err(|failure| format!("{failure:?}"))
        .check("a tree")?;
    let mut pending = vec![tree.root_node()];
    let mut macro_node = None;
    while let Some(node) = pending.pop() {
        if node.kind() == "macro_invocation" {
            macro_node = Some(node);
            break;
        }
        crate::evidence::push_children(&mut pending, &node, |child| child);
    }
    let macro_node = macro_node.check("the fixture has a macro")?;
    let spent = Deadline::after(std::time::Duration::ZERO);
    let input = WalkInput {
        source,
        lines: &lines,
        deadline: Some(&spent),
    };

    let recovered = super::macro_calls::extract(&macro_node, &input, Some("run"))
        .check("recover the macro's calls")?;

    assert!(recovered.timed_out);
    assert!(!recovered.capped);
    assert!(recovered.rows.is_empty(), "helper() was not recovered");
    Ok(())
}
