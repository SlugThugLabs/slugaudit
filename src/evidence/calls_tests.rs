use crate::evidence::{Extraction, WalkInput};
use crate::model::{CallPayload, EvidenceItem, EvidenceKind, EvidencePayload, LineIndex};
use crate::util::Deadline;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

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

fn extract_rust(source: &str) -> TestResult<Vec<EvidenceItem>> {
    Ok(walk_rust(source, None)?.items)
}

fn call_payload(item: &EvidenceItem) -> TestResult<&CallPayload> {
    match &item.payload {
        EvidencePayload::Call(payload) => Ok(payload),
        other => Err(format!("expected CallPayload, got {other:?}")).check("call payload"),
    }
}

fn callers_of<'a>(calls: &'a [EvidenceItem], callee: &str) -> TestResult<Vec<Option<&'a str>>> {
    let mut out = Vec::new();
    for item in calls {
        let payload = call_payload(item)?;
        if payload.callee_name == callee {
            out.push(payload.caller.as_deref());
        }
    }
    Ok(out)
}

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
        .map(|item| Ok(call_payload(item)?.callee_name.as_str()))
        .collect::<TestResult<_>>()?;
    ensure(names.contains(&"helper"), "names.contains(&\"helper\")")?;
    ensure(names.contains(&"start"), "names.contains(&\"start\")")?;
    ensure(names.contains(&"println"), "names.contains(&\"println\")")?;

    let run_call = call_payload(
        calls
            .iter()
            .find(|item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == "helper"))
            .check("expected run_call helper")?,
    )?;
    ensure_eq(&run_call.caller.as_deref(), &Some("run"), "caller == run")?;
    ensure_eq(
        &run_call.resolution.as_str(),
        &"unresolved-syntactic",
        "resolution == unresolved-syntactic",
    )?;

    let macro_call = call_payload(
        calls
            .iter()
            .find(|item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == "println"))
            .check("expected macro_call println")?,
    )?;
    ensure_eq(
        &macro_call.resolution.as_str(),
        &"macro",
        "macro_call resolution == macro",
    )?;
    Ok(())
}

#[test]
fn calls_have_spans_and_do_not_include_definition_names() -> TestResult {
    let source = "fn build() { make(); }\nfn make() {}\n";
    let calls = extract_rust(source)?;
    ensure_eq(&calls.len(), &1, "calls.len() == 1")?;
    let call_item = calls.first().check("calls[0] exists")?;
    let payload = call_payload(call_item)?;
    ensure_eq(&payload.callee_text.as_str(), &"make", "callee_text")?;
    ensure_eq(&payload.caller.as_deref(), &Some("build"), "caller")?;
    ensure(
        matches!(call_item.span, crate::model::SpanAvailability::Present(_)),
        "call span present",
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
            let p = call_payload(item)?;
            Ok((
                p.callee_name.as_str(),
                p.callee_text.as_str(),
                p.resolution.as_str(),
            ))
        })
        .collect::<TestResult<_>>()?;
    ensure_eq(
        &named,
        &[
            ("foo", "foo::<u8>", "unresolved-syntactic"),
            ("parse", "x.parse::<u32>", "unresolved-syntactic"),
        ],
        "turbofish inner callee names",
    )?;
    Ok(())
}

#[test]
fn a_qualified_turbofish_callee_stays_qualified() -> TestResult {
    let source = "fn run() { std::mem::size_of::<u8>(); }\n";
    let calls = extract_rust(source)?;
    ensure_eq(&calls.len(), &1, format!("{calls:?}"))?;
    let payload = call_payload(calls.first().check("calls[0] exists")?)?;
    ensure_eq(&payload.callee_name.as_str(), &"size_of", "callee_name")?;
    ensure_eq(
        &payload.resolution.as_str(),
        &"qualified-syntactic",
        "resolution",
    )?;
    ensure_eq(&payload.confidence.as_str(), &"high", "confidence")?;
    Ok(())
}

#[test]
fn a_qualified_call_inside_a_macro_has_the_same_confidence_as_outside() -> TestResult {
    let source = "fn run() { println!(\"{}\", crate::util::now()); }\n";
    let calls = extract_rust(source)?;

    let now = call_payload(
        calls
            .iter()
            .find(
                |item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == "now"),
            )
            .check("the macro argument call is recovered")?,
    )?;
    ensure_eq(
        &now.resolution.as_str(),
        &"qualified-syntactic",
        "resolution",
    )?;
    ensure_eq(&now.confidence.as_str(), &"high", "confidence")?;
    Ok(())
}

#[test]
fn malformed_rust_does_not_panic() -> TestResult {
    let calls = extract_rust("fn run() { helper(\n")?;
    ensure(calls.len() <= 1, "calls.len() <= 1")?;
    Ok(())
}

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
    let hits: Vec<&EvidenceItem> = calls
        .iter()
        .filter(
            |item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == "unwrap"),
        )
        .collect();
    ensure_eq(&hits.len(), &8, format!("{calls:?}"))?;
    for call in &hits {
        let payload = call_payload(call)?;
        ensure_eq(&payload.caller.as_deref(), &Some("run"), "caller == run")?;
        ensure(
            matches!(call.span, crate::model::SpanAvailability::Present(_)),
            "span present",
        )?;
        ensure(
            payload.callee_text.ends_with("unwrap"),
            format!("callee_text {}", payload.callee_text),
        )?;
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

#[test]
fn a_call_in_a_closure_has_no_caller_inside_and_outside_a_macro() -> TestResult {
    let source = "fn run(v: Vec<u8>) {\n    \
                  v.iter().for_each(|x| plain(x));\n    \
                  println!(\"{:?}\", v.iter().map(|x| wrapped(x)));\n    \
                  direct();\n    \
                  println!(\"{}\", in_macro());\n}\n";
    let calls = extract_rust(source)?;

    ensure_eq(&callers_of(&calls, "plain")?, &[None], format!("{calls:?}"))?;
    ensure_eq(
        &callers_of(&calls, "wrapped")?,
        &[None],
        format!("{calls:?}"),
    )?;
    ensure_eq(
        &callers_of(&calls, "direct")?,
        &[Some("run")],
        format!("{calls:?}"),
    )?;
    ensure_eq(
        &callers_of(&calls, "in_macro")?,
        &[Some("run")],
        format!("{calls:?}"),
    )?;
    ensure_eq(
        &callers_of(&calls, "for_each")?,
        &[Some("run")],
        format!("{calls:?}"),
    )?;
    ensure_eq(
        &callers_of(&calls, "map")?,
        &[Some("run")],
        format!("{calls:?}"),
    )?;
    Ok(())
}

#[test]
fn a_call_reports_the_innermost_enclosing_function() -> TestResult {
    let source = "fn outer() {\n    \
                  fn inner() { deep(); }\n    \
                  shallow();\n}\n\
                  impl T { fn method(&self) { in_method(); } }\n\
                  const C: u8 = top();\n";
    let calls = extract_rust(source)?;

    ensure_eq(&callers_of(&calls, "deep")?, &[Some("inner")], "deep")?;
    ensure_eq(&callers_of(&calls, "shallow")?, &[Some("outer")], "shallow")?;
    ensure_eq(
        &callers_of(&calls, "in_method")?,
        &[Some("method")],
        "in_method",
    )?;
    ensure_eq(&callers_of(&calls, "top")?, &[None], "top")?;
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
        crate::evidence::push_children(&mut pending, node, |child| child);
    }
    let macro_node = macro_node.check("the fixture has a macro")?;
    let spent = Deadline::after(std::time::Duration::ZERO);
    let input = WalkInput {
        source,
        lines: &lines,
        deadline: Some(&spent),
    };

    let grammar = tree.language();
    let mut parser_slot = None;
    let recovered =
        super::macro_calls::extract(macro_node, &input, Some("run"), &grammar, &mut parser_slot)
            .check("recover calls")?;

    ensure(recovered.timed_out, "recovered.timed_out")?;
    ensure(!recovered.capped, "!recovered.capped")?;
    ensure(recovered.rows.is_empty(), "helper() was not recovered")?;
    Ok(())
}

#[test]
fn statement_and_item_macros_recover_nested_calls_with_exact_source_spans() -> TestResult {
    let source = concat!(
        "thread_local! {\n",
        "    static SLOT: Cell<u32> = Cell::new(init_slot());\n",
        "}\n",
        "decl_fn! {\n",
        "    fn generated() {\n",
        "        inside_generated();\n",
        "    }\n",
        "}\n",
        "fn run() {\n",
        "    block_macro! {\n",
        "        let first = step_one();\n",
        "        crate::pipeline::step_two(first);\n",
        "    }\n",
        "}\n",
    );
    let calls = extract_rust(source)?;

    for (callee, expected_caller, expected_res, expected_line) in [
        ("new", None, "qualified-syntactic", 2_i64),
        ("init_slot", None, "unresolved-syntactic", 2_i64),
        (
            "inside_generated",
            Some("generated"),
            "unresolved-syntactic",
            6_i64,
        ),
        ("step_one", Some("run"), "unresolved-syntactic", 11_i64),
        ("step_two", Some("run"), "qualified-syntactic", 12_i64),
    ] {
        let hit = calls
            .iter()
            .find(
                |item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == callee),
            )
            .check(&format!("recovered macro call `{callee}`"))?;
        let payload = call_payload(hit)?;
        ensure_eq(
            &payload.caller.as_deref(),
            &expected_caller,
            format!("caller for `{callee}`"),
        )?;
        ensure_eq(
            &payload.resolution.as_str(),
            &expected_res,
            format!("resolution for `{callee}`"),
        )?;
        let row = crate::evidence::to_row(hit);
        ensure_eq(
            &row.start_line,
            &Some(expected_line),
            format!("1-based start_line for `{callee}`"),
        )?;
    }
    Ok(())
}

#[test]
fn string_literal_receiver_with_double_colon_is_not_classified_as_qualified_path() -> TestResult {
    let source = "fn run() { let _ = \"a::b\".trim(); }\n";
    let calls = extract_rust(source)?;
    ensure_eq(&calls.len(), &1usize, "single trim() call")?;
    let payload = call_payload(calls.first().check("trim call present")?)?;
    ensure_eq(&payload.callee_name.as_str(), &"trim", "callee_name")?;
    ensure_eq(
        &payload.resolution.as_str(),
        &"unresolved-syntactic",
        "method call on string literal containing :: is unresolved-syntactic",
    )?;
    Ok(())
}

#[test]
fn qualified_receiver_method_calls_are_unresolved_syntactic() -> TestResult {
    let source = "fn run() {\n    Foo::new().expect(\"ok\");\n    crate::get().unwrap();\n}\n";
    let calls = extract_rust(source)?;
    for (name, expected_text, expected_res) in [
        ("new", "Foo::new", "qualified-syntactic"),
        ("expect", "Foo::new().expect", "unresolved-syntactic"),
        ("get", "crate::get", "qualified-syntactic"),
        ("unwrap", "crate::get().unwrap", "unresolved-syntactic"),
    ] {
        let hit = calls
            .iter()
            .find(|item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == name))
            .check(&format!("expected call `{name}`"))?;
        let payload = call_payload(hit)?;
        ensure_eq(&payload.callee_text.as_str(), &expected_text, name)?;
        ensure_eq(&payload.resolution.as_str(), &expected_res, name)?;
    }
    Ok(())
}

#[test]
fn nested_items_inside_function_do_not_inherit_enclosing_caller() -> TestResult {
    let source = concat!(
        "fn outer() {\n",
        "    static S: usize = from_static();\n",
        "    const C: usize = from_const();\n",
        "    mod m { const M: usize = from_mod(); fn inner() { from_inner(); } }\n",
        "    direct();\n",
        "}\n",
    );
    let calls = extract_rust(source)?;
    ensure_eq(&callers_of(&calls, "from_static")?, &[None], "static")?;
    ensure_eq(&callers_of(&calls, "from_const")?, &[None], "const")?;
    ensure_eq(&callers_of(&calls, "from_mod")?, &[None], "mod")?;
    ensure_eq(
        &callers_of(&calls, "from_inner")?,
        &[Some("inner")],
        "inner fn in mod",
    )?;
    ensure_eq(&callers_of(&calls, "direct")?, &[Some("outer")], "direct")?;
    Ok(())
}

#[test]
fn parenthesized_indexed_and_commented_callees_keep_clean_names() -> TestResult {
    let source = concat!(
        "fn run(arr: [fn(u8); 1], x: u8) {\n",
        "    (get_fn())(x);\n",
        "    arr[0](x);\n",
        "    foo::bar /* comment */ ();\n",
        "}\n",
    );
    let calls = extract_rust(source)?;
    let paren = calls
        .iter()
        .find(|item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_text == "(get_fn())"))
        .check("parenthesized callee without leaked args")?;
    ensure_eq(
        &call_payload(paren)?.callee_name.as_str(),
        &"(get_fn())",
        "paren",
    )?;
    let indexed = calls
        .iter()
        .find(|item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_text == "arr[0]"))
        .check("indexed callee without leaked args")?;
    ensure_eq(
        &call_payload(indexed)?.callee_name.as_str(),
        &"arr[0]",
        "indexed",
    )?;
    let commented = calls
        .iter()
        .find(|item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == "bar"))
        .check("commented scoped callee uses name field")?;
    ensure_eq(
        &call_payload(commented)?.resolution.as_str(),
        &"qualified-syntactic",
        "commented qualified",
    )?;
    Ok(())
}

#[test]
fn unclosed_utf8_macro_invocation_does_not_truncate_or_chop_utf8() -> TestResult {
    let extraction = walk_rust("fn run() { println!(\"é", None)?;
    ensure(
        !extraction.truncated,
        "unclosed UTF-8 macro must not truncate",
    )?;
    ensure(
        !extraction.timed_out,
        "unclosed UTF-8 macro must not time out",
    )?;
    Ok(())
}

#[test]
fn raw_string_and_char_literal_is_qualified_checks() -> TestResult {
    use super::site::is_qualified;
    for unqualified in [
        r#"r"a::b".trim"#,
        r##"r#"a"b::c"#.trim"##,
        r##"br#"a::b"#.trim"##,
        r#"('"', "a::b")"#,
        r#"('\'', "a::b")"#,
        r"b'\''.is_ascii",
    ] {
        ensure(
            !is_qualified(unqualified),
            format!("expected false for {unqualified}"),
        )?;
    }
    for qualified in [
        r##"r#"a::b"#::call"##,
        r#"('"', crate::foo)"#,
        r"('\'', crate::foo)",
        "r#match::call",
        "Foo::<'a>::bar",
    ] {
        ensure(
            is_qualified(qualified),
            format!("expected true for {qualified}"),
        )?;
    }
    Ok(())
}
