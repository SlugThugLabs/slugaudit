use crate::evidence::tests::field;
use crate::evidence::{Extraction, WalkInput};
use crate::model::LineIndex;
use crate::util::Deadline;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

/// Parses `source` (unbounded) and runs the Rust call walker over it within
/// `deadline`.
fn walk_rust(source: &str, deadline: Option<&Deadline>) -> TestResult<Extraction> {
    let tree = crate::evidence::parse_tree("rust", source, None)
        .map_err(|failure| format!("{failure:?}"))
        .check("parse the Rust source")?;
    let lines = LineIndex::new(source).check("index source")?;
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
        .filter(|item| *field(&item.payload, "callee_name") == callee)
        .map(|item| field(&item.payload, "caller"))
        .collect()
}
use crate::model::EvidenceKind;

#[test]
fn extracts_direct_qualified_and_macro_calls() -> TestResult {
    let source = "fn run() {\n    helper();\n    crate::worker::start();\n    println!(\"ok\");\n}\nfn helper() {}\n";
    let calls = extract_rust(source)?;
    ensure_eq(&calls.len(), &3, "calls.len() == 3")?;
    ensure(
        calls.iter().all(|item| item.kind == EvidenceKind::Call),
        "calls.iter().all(|item| item.kind == EvidenceKind::Call)",
    )?;

    let names: Vec<&str> = calls
        .iter()
        .map(|item| {
            field(&item.payload, "callee_name")
                .as_str()
                .check("callee_name as string")
        })
        .collect::<TestResult<_>>()?;
    ensure(names.contains(&"helper"), "names.contains(&\"helper\")")?;
    ensure(names.contains(&"start"), "names.contains(&\"start\")")?;
    ensure(names.contains(&"println"), "names.contains(&\"println\")")?;

    let run_call = calls
        .iter()
        .find(|item| *field(&item.payload, "callee_name") == "helper")
        .check("expected run_call helper")?;
    ensure_eq(
        field(&run_call.payload, "caller"),
        &"run",
        "run_call.payload[\"caller\"] == \"run\"",
    )?;
    ensure_eq(
        field(&run_call.payload, "resolution"),
        &"unresolved-syntactic",
        "run_call.payload[\"resolution\"] == \"unresolved-syntactic\"",
    )?;

    let macro_call = calls
        .iter()
        .find(|item| *field(&item.payload, "callee_name") == "println")
        .check("expected macro_call println")?;
    ensure_eq(
        field(&macro_call.payload, "resolution"),
        &"macro",
        "macro_call.payload[\"resolution\"] == \"macro\"",
    )?;
    Ok(())
}

#[test]
fn calls_have_spans_and_do_not_include_definition_names() -> TestResult {
    let source = "fn build() { make(); }\nfn make() {}\n";
    let calls = extract_rust(source)?;
    ensure_eq(&calls.len(), &1, "calls.len() == 1")?;
    let call = &calls.first().check("calls[0] exists")?;
    ensure_eq(
        field(&call.payload, "callee_text"),
        &"make",
        "call.payload[\"callee_text\"] == \"make\"",
    )?;
    ensure_eq(
        field(&call.payload, "caller"),
        &"build",
        "call.payload[\"caller\"] == \"build\"",
    )?;
    ensure(
        matches!(call.span, crate::model::SpanAvailability::Present(_)),
        "matches!( call.span, crate::model::SpanAvailability::Present(_) )",
    )?;
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
                field(&item.payload, "callee_name")
                    .as_str()
                    .check("callee_name")?,
                field(&item.payload, "callee_text")
                    .as_str()
                    .check("callee_text")?,
                field(&item.payload, "resolution")
                    .as_str()
                    .check("resolution")?,
            ))
        })
        .collect::<TestResult<_>>()?;
    ensure_eq(
        &named,
        &[
            ("foo", "foo::<u8>", "unresolved-syntactic"),
            ("parse", "x.parse::<u32>", "unresolved-syntactic"),
        ],
        "named == [ (\"foo\", \"foo::<u8>\", \"unresolved-syntactic\"), (\"parse\", \"x.parse::<u32>\", \"unresolved-syntactic\"), ]",
    )?;
    Ok(())
}

#[test]
fn a_qualified_turbofish_callee_stays_qualified() -> TestResult {
    let source = "fn run() { std::mem::size_of::<u8>(); }\n";

    let calls = extract_rust(source)?;

    ensure_eq(&calls.len(), &1, format!("{calls:?}"))?;
    ensure_eq(
        field(
            &calls.first().check("calls[0] exists")?.payload,
            "callee_name",
        ),
        &"size_of",
        "calls[0].payload[\"callee_name\"] == \"size_of\"",
    )?;
    ensure_eq(
        field(
            &calls.first().check("calls[0] exists")?.payload,
            "resolution",
        ),
        &"qualified-syntactic",
        "calls[0].payload[\"resolution\"] == \"qualified-syntactic\"",
    )?;
    ensure_eq(
        field(
            &calls.first().check("calls[0] exists")?.payload,
            "confidence",
        ),
        &"high",
        "calls[0].payload[\"confidence\"] == \"high\"",
    )?;
    Ok(())
}

#[test]
fn a_qualified_call_inside_a_macro_has_the_same_confidence_as_outside() -> TestResult {
    let source = "fn run() { println!(\"{}\", crate::util::now()); }\n";

    let calls = extract_rust(source)?;

    let now = calls
        .iter()
        .find(|item| *field(&item.payload, "callee_name") == "now")
        .check("the macro argument call is recovered")?;
    ensure_eq(
        field(&now.payload, "resolution"),
        &"qualified-syntactic",
        "now.payload[\"resolution\"] == \"qualified-syntactic\"",
    )?;
    ensure_eq(
        field(&now.payload, "confidence"),
        &"high",
        "now.payload[\"confidence\"] == \"high\"",
    )?;
    Ok(())
}

#[test]
fn malformed_rust_does_not_panic() -> TestResult {
    let calls = extract_rust("fn run() { helper(\n")?;
    ensure(calls.len() <= 1, "calls.len() <= 1")?;
    Ok(())
}

/// The one fixture with a callee named `unwrap`: a method call reports its
/// field name as `callee_name` (keeping the receiver in `callee_text`), a call
/// inside a macro's arguments is found exactly once (also when macros nest),
/// and text inside a string or a comment is never a call.
#[test]
fn method_calls_are_found_once_each_in_code_and_macro_arguments_and_not_in_text() -> TestResult {
    let source = r#"
fn run(a: Option<u8>, b: Option<u8>) -> u8 {
    assert_eq!(a.unwrap(), b.unwrap());
    debug_assert!(Some(2).unwrap() > 0);
    println!("fake.unwrap() {}", a.unwrap()); // ignored.unwrap()
    assert!(matches!(b.unwrap(), 1), "text.unwrap()");
    let _ = vec![Some(4).unwrap(), Some(5).unwrap()];
    a.unwrap()
}
"#;
    let calls = extract_rust(source)?;
    let hits: Vec<_> = calls
        .iter()
        .filter(|item| *field(&item.payload, "callee_name") == "unwrap")
        .collect();
    ensure_eq(&hits.len(), &8, format!("{calls:?}"))?;
    for call in &hits {
        ensure_eq(
            field(&call.payload, "caller"),
            &"run",
            "call.payload[\"caller\"] == \"run\"",
        )?;
        ensure(
            matches!(call.span, crate::model::SpanAvailability::Present(_)),
            "matches!( call.span, crate::model::SpanAvailability::Present(_) )",
        )?;
        let text = field(&call.payload, "callee_text")
            .as_str()
            .check("callee_text")?;
        ensure(text.ends_with("unwrap"), format!("callee_text {text}"))?;
    }
    Ok(())
}

#[test]
fn a_call_walk_under_a_spent_deadline_is_timed_out_not_truncated() -> TestResult {
    let source = "fn run() { helper(); }\n";
    let spent = Deadline::after(std::time::Duration::ZERO);

    let extraction = walk_rust(source, Some(&spent))?;

    ensure(extraction.items.is_empty(), "extraction.items.is_empty()")?;
    ensure(extraction.timed_out, "extraction.timed_out")?;
    ensure(!extraction.truncated, "!extraction.truncated")?;
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
    ensure_eq(&callers_of(&calls, "plain"), &[&null], format!("{calls:?}"))?;
    ensure_eq(
        &callers_of(&calls, "wrapped"),
        &[&null],
        format!("{calls:?}"),
    )?;
    ensure_eq(&callers_of(&calls, "direct"), &[&run], format!("{calls:?}"))?;
    ensure_eq(
        &callers_of(&calls, "in_macro"),
        &[&run],
        format!("{calls:?}"),
    )?;
    ensure_eq(
        &callers_of(&calls, "for_each"),
        &[&run],
        format!("{calls:?}"),
    )?;
    ensure_eq(&callers_of(&calls, "map"), &[&run], format!("{calls:?}"))?;
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

    ensure_eq(
        &callers_of(&calls, "deep"),
        &[&serde_json::Value::from("inner")],
        "callers_of(&calls, \"deep\") == [&serde_json::Value::from(\"inner\")]",
    )?;
    ensure_eq(
        &callers_of(&calls, "shallow"),
        &[&serde_json::Value::from("outer")],
        "callers_of(&calls, \"shallow\") == [&serde_json::Value::from(\"outer\")]",
    )?;
    ensure_eq(
        &callers_of(&calls, "in_method"),
        &[&serde_json::Value::from("method")],
        "callers_of(&calls, \"in_method\") == [&serde_json::Value::from(\"method\")]",
    )?;
    ensure_eq(
        &callers_of(&calls, "top"),
        &[&serde_json::Value::Null],
        "callers_of(&calls, \"top\") == [&serde_json::Value::Null]",
    )?;
    Ok(())
}

#[test]
fn a_macro_body_re_parse_under_a_spent_deadline_is_timed_out() -> TestResult {
    let source = "fn run() { println!(\"{}\", helper()); }\n";
    let lines = LineIndex::new(source).check("index source")?;
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

    ensure(recovered.timed_out, "recovered.timed_out")?;
    ensure(!recovered.capped, "!recovered.capped")?;
    ensure(recovered.rows.is_empty(), "helper() was not recovered")?;
    Ok(())
}
