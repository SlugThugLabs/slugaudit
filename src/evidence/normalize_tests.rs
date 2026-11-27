use super::*;
use crate::model::{EvidenceKind, EvidenceOrigin, EvidencePayload};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

fn extract_items(language: &str, source: &str) -> TestResult<Vec<EvidenceItem>> {
    Ok(extract(language, source).check("extract")?.0)
}

fn kinds(items: &[EvidenceItem]) -> Vec<&EvidenceKind> {
    items.iter().map(|item| &item.kind).collect()
}

#[test]
fn extracts_structure_imports_symbols_and_comments_from_rust() -> TestResult {
    let source = "use std::io;\n\n\
/// Say hello.\n\
pub fn greet(name: &str) -> &str {\n    \
    // a line comment\n    \
    let message = name;\n    \
    message\n\
}\n";
    let items = extract_items("rust", source)?;

    ensure(
        kinds(&items).contains(&&EvidenceKind::Structure),
        "has Structure",
    )?;
    ensure(kinds(&items).contains(&&EvidenceKind::Import), "has Import")?;
    ensure(
        kinds(&items).contains(&&EvidenceKind::Comment),
        "has Comment",
    )?;
    ensure(kinds(&items).contains(&&EvidenceKind::Symbol), "has Symbol")?;

    let structure = items
        .iter()
        .find_map(|item| match &item.payload {
            EvidencePayload::Structure(s) if s.name.as_deref() == Some("greet") => Some(s),
            _ => None,
        })
        .check("Structure greet present")?;
    ensure_eq(&structure.kind.as_str(), &"Function", "greet is Function")?;
    ensure_eq(
        &structure.doc_comment.as_deref(),
        &Some("/// Say hello."),
        "greet doc_comment",
    )?;
    Ok(())
}

#[test]
fn extracts_rust_structure_and_a_nested_child_gets_a_distinct_key() -> TestResult {
    let source = "pub struct Widget {\n}\n\nimpl Widget {\n    pub fn build(&self) {}\n}\n";
    let items = extract_items("rust", source)?;

    let structure_keys: Vec<&str> = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Structure)
        .map(|item| item.key.as_str())
        .collect();
    ensure(
        structure_keys.len() >= 3,
        "expected struct + impl + method structure items",
    )?;
    ensure_eq(
        &structure_keys.len(),
        &(structure_keys
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()),
        "every structure key is distinct",
    )?;
    Ok(())
}

#[test]
fn a_source_nested_past_the_walk_bound_is_truncated_not_aborted() -> TestResult {
    let nesting = usize::try_from(crate::evidence::MAX_WALK_DEPTH).check("fits")? * 4;
    let source = format!(
        "fn deep() {{ let x = {}1{}; }}\n",
        "(".repeat(nesting),
        ")".repeat(nesting)
    );

    let (items, depth_capped) =
        extract("rust", &source).check("a deep source is truncated, not a failure")?;

    ensure(
        depth_capped,
        "a walk that stopped at the bound must report it",
    )?;
    ensure(!items.is_empty(), "the walk still yields what it reached")?;
    Ok(())
}

#[test]
fn a_rust_macro_nested_past_the_macro_bound_is_reported_partial_not_aborted() -> TestResult {
    let nesting = 10_000;
    let source = format!(
        "fn run() {{ {}1{} }}",
        "a!(".repeat(nesting),
        ")".repeat(nesting)
    );

    let (items, depth_capped) =
        extract("rust", &source).check("a deep macro is truncated, not a failure")?;

    ensure(depth_capped, "a macro scan cut short must be reported")?;
    ensure(!items.is_empty(), "the walk still yields what it reached")?;
    Ok(())
}

#[test]
fn a_rust_macro_nested_within_the_bound_is_fully_scanned() -> TestResult {
    let source = "fn run(v: Option<u8>) { a!(b!(c!(v.expect()))); }";

    let (items, depth_capped) = extract("rust", source).check("extract")?;

    ensure(!depth_capped, "!depth_capped")?;
    ensure(
        items.iter().any(
            |item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == "expect"),
        ),
        "expect call inside nested macro recovered",
    )?;
    Ok(())
}

#[test]
fn a_multi_megabyte_single_line_file_indexes_in_linear_time() -> TestResult {
    let pad = "é".repeat(8 * 1024 * 1024);
    let source = format!("fn r() {{ /*{pad}*/ {}}}", "helper(1); ".repeat(20_000));
    let started = std::time::Instant::now();

    let (items, _) = extract("rust", &source).check("extract")?;

    let calls = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Call)
        .count();
    ensure_eq(&calls, &20_000, "calls == 20_000")?;
    ensure(
        started.elapsed() < std::time::Duration::from_secs(20),
        format!("indexing took {:?}", started.elapsed()),
    )?;
    Ok(())
}

#[test]
fn malformed_source_still_produces_diagnostics() -> TestResult {
    let items = extract_items("rust", "fn broken( {\n")?;
    let diagnostics: Vec<_> = items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Diagnostic(d) => Some(d),
            _ => None,
        })
        .collect();
    ensure(!diagnostics.is_empty(), "diagnostics emitted")?;
    ensure(
        diagnostics.iter().all(|d| d.severity == "error"),
        "all syntax error diagnostics have error severity",
    )?;
    Ok(())
}

#[test]
fn every_item_carries_a_span_or_an_explicit_missing_reason() -> TestResult {
    let source = "fn a() {}\nfn b() {}\n";
    let items = extract_items("rust", source)?;
    ensure(!items.is_empty(), "!items.is_empty()")?;
    for item in &items {
        match &item.span {
            SpanAvailability::Present(span) => ensure(
                span.end_byte >= span.start_byte,
                "span.end_byte >= span.start_byte",
            )?,
            SpanAvailability::NormalizerUnavailable
            | SpanAvailability::PackOmitted
            | SpanAvailability::DerivedEvidence => {}
        }
    }
    Ok(())
}

#[test]
fn unknown_language_is_a_typed_error_not_a_panic() -> TestResult {
    let err =
        extract("not-a-real-language", "irrelevant").check_err("unknown language should fail")?;
    ensure(
        matches!(
            err,
            ExtractError::Pack(PackError::Download(_) | PackError::LanguageNotFound(_))
        ),
        format!("expected Download or LanguageNotFound error, got: {err:?}"),
    )?;
    Ok(())
}

#[test]
fn rust_let_bindings_surface_as_variable_symbols() -> TestResult {
    let source =
        "fn pick(first: u32, second: u32) -> u32 {\n    let winner = first;\n    winner\n}\n";
    let items = extract_items("rust", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.kind == "Variable" => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    ensure(
        names.contains(&"winner"),
        format!("expected the let binding `winner`, got {names:?}"),
    )?;
    Ok(())
}

#[test]
fn rust_parameters_surface_as_variable_symbols() -> TestResult {
    let source = "fn greet(name: &str, age: u8) {}\n";
    let items = extract_items("rust", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter(|item| item.origin == EvidenceOrigin::GenericWalker)
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.kind == "Variable" => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    ensure(names.contains(&"name"), format!("got {names:?}"))?;
    ensure(names.contains(&"age"), format!("got {names:?}"))?;
    Ok(())
}

#[test]
fn rust_struct_fields_surface_as_field_symbols() -> TestResult {
    let source = "struct Person {\n    name: String,\n    age: u8,\n}\n";
    let items = extract_items("rust", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.kind == r#"Other("Field")"# => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    ensure(names.contains(&"name"), format!("got {names:?}"))?;
    ensure(names.contains(&"age"), format!("got {names:?}"))?;
    Ok(())
}

#[test]
fn binding_names_are_sliced_from_source_not_hardcoded() -> TestResult {
    let source = "fn index(file_path: &str, filepath: &str) {\n    let file_path_len = file_path.len();\n    let filepath_len = filepath.len();\n}\n";
    let items = extract_items("rust", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    ensure(names.contains(&"file_path"), format!("got {names:?}"))?;
    ensure(names.contains(&"filepath"), format!("got {names:?}"))?;
    ensure(names.contains(&"file_path_len"), format!("got {names:?}"))?;
    ensure(names.contains(&"filepath_len"), format!("got {names:?}"))?;
    Ok(())
}

#[test]
fn rust_control_flow_and_closure_bindings_surface_as_variable_symbols() -> TestResult {
    let source = concat!(
        "fn run(opt: Option<u32>, items: &[u32]) {\n",
        "    if let Some(found_val) = opt {\n",
        "        for (loop_idx, loop_item) in items.iter().enumerate() {\n",
        "            let f = |closure_typed: u32, (closure_a, closure_b)| {\n",
        "                closure_typed + found_val + loop_idx as u32 + *loop_item + closure_a + closure_b\n",
        "            };\n",
        "            let _ = f(1, (2, 3));\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let items = extract_items("rust", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter(|item| item.origin == EvidenceOrigin::GenericWalker)
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.kind == "Variable" => Some(s.name.as_str()),
            _ => None,
        })
        .collect();

    for expected in [
        "found_val",
        "loop_idx",
        "loop_item",
        "closure_typed",
        "closure_a",
        "closure_b",
    ] {
        ensure(
            names.contains(&expected),
            format!("missing `{expected}` in {names:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn rust_imports_never_double_report_via_the_generic_walker() -> TestResult {
    let source = "use std::io;\nuse crate::sibling::*;\nextern crate alloc;\n";
    let items = extract_items("rust", source)?;
    let imports: Vec<(&EvidenceOrigin, &str, bool)> = items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Import(imp) => {
                Some((&item.origin, imp.source.as_str(), imp.is_wildcard))
            }
            _ => None,
        })
        .collect();
    ensure_eq(
        &imports,
        &vec![
            (&EvidenceOrigin::PackStructure, "use std::io;", false),
            (
                &EvidenceOrigin::PackStructure,
                "use crate::sibling::*;",
                true,
            ),
            (&EvidenceOrigin::PackStructure, "extern crate alloc;", false),
        ],
        "Rust single-parse CST extractor emits exact imports once each",
    )?;
    Ok(())
}

#[test]
fn binding_extraction_yields_zero_for_comment_only_rust_source() -> TestResult {
    let source = "// just a comment, no bindings anywhere\n";
    let items = extract_items("rust", source)?;
    let binding_count = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol && item.origin == EvidenceOrigin::GenericWalker
        })
        .count();
    ensure_eq(&binding_count, &0, "no bindings to extract here")?;
    Ok(())
}

#[test]
fn a_walker_parser_failure_marks_the_extraction_truncated() -> TestResult {
    let lines = LineIndex::new("x").check("index source")?;
    let input = WalkInput {
        source: "x",
        lines: &lines,
        deadline: None,
    };

    let extraction = extract_extra_walkers("no-such-language-for-walker-test", &input, true)
        .check("run the walkers")?;

    ensure(extraction.items.is_empty(), "extraction.items.is_empty()")?;
    ensure(!extraction.timed_out, "no deadline, so no timeout")?;
    ensure(
        extraction.truncated,
        "a failed parser setup must not read as a complete extraction",
    )?;
    Ok(())
}

const WIDE_NODE_ELEMENTS: usize = 30_000;

fn wide_node_source() -> String {
    let elements = "f(1), ".repeat(WIDE_NODE_ELEMENTS);
    format!("fn run() {{\n    let a = [{elements}];\n    let b = vec![{elements}];\n}}\n")
}

#[test]
fn a_wide_rust_node_is_walked_in_linear_time() -> TestResult {
    let source = wide_node_source();
    let lines = LineIndex::new(&source).check("index source")?;
    let deadline = Deadline::after(std::time::Duration::from_secs(10));
    let input = WalkInput {
        source: &source,
        lines: &lines,
        deadline: Some(&deadline),
    };
    let started = std::time::Instant::now();

    let extraction = extract_extra_walkers("rust", &input, true).check("run the walkers")?;

    let elapsed = started.elapsed();
    ensure(
        !extraction.timed_out,
        format!("the walk timed out after {elapsed:?}"),
    )?;
    ensure(!extraction.truncated, "!extraction.truncated")?;
    let calls: Vec<_> = extraction
        .items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Call(c) => Some(c),
            _ => None,
        })
        .collect();
    ensure_eq(
        &calls.len(),
        &(2 * WIDE_NODE_ELEMENTS + 1),
        "calls.len() == 2 * WIDE_NODE_ELEMENTS + 1",
    )?;
    ensure(
        calls
            .iter()
            .all(|call| call.caller.as_deref() == Some("run")),
        "every call has caller run",
    )?;
    let bindings: Vec<&str> = extraction
        .items
        .iter()
        .filter(|item| item.origin == EvidenceOrigin::GenericWalker)
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) => Some(s.name.as_str()),
            _ => None,
        })
        .collect();
    ensure_eq(&bindings, &["a", "b"], "bindings == [\"a\", \"b\"]")?;
    ensure(
        elapsed < std::time::Duration::from_secs(5),
        format!("walking {WIDE_NODE_ELEMENTS}-wide nodes took {elapsed:?}"),
    )?;
    Ok(())
}

#[test]
fn a_walk_past_its_deadline_is_timed_out_not_truncated() -> TestResult {
    let source = "fn run() { let a = helper(); }\n";
    let lines = LineIndex::new(source).check("index source")?;
    let tree = crate::evidence::parse_tree("rust", source, None)
        .map_err(|failure| format!("{failure:?}"))
        .check("parse the source")?;
    let spent = Deadline::after(std::time::Duration::ZERO);
    let input = WalkInput {
        source,
        lines: &lines,
        deadline: Some(&spent),
    };

    let out = crate::evidence::rust_items::extract_rust_cst(&tree, &input)
        .check("walk Rust CST under spent deadline")?;

    ensure(out.items.is_empty(), format!("{:?}", out.items))?;
    ensure(out.timed_out, "out.timed_out")?;
    ensure(!out.truncated, "!out.truncated")?;
    Ok(())
}

#[path = "normalize_stack_tests.rs"]
mod stack;
