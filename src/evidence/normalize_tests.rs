use super::*;
use crate::evidence::tests::field;
use crate::model::{EvidenceKind, EvidenceOrigin};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

/// `extract` also reports whether a walker stopped at the depth cap. These
/// tests are about the items, so they take the first half; the cap has its
/// own test in this module.
fn extract_items(language: &str, source: &str) -> TestResult<Vec<EvidenceItem>> {
    Ok(extract(language, source).check("extract")?.0)
}

fn kinds(items: &[EvidenceItem]) -> Vec<&EvidenceKind> {
    items.iter().map(|item| &item.kind).collect()
}

#[test]
fn extracts_structure_imports_symbols_and_comments_from_python() -> TestResult {
    let source = "import os\n\n\
def greet(name):\n    \
\"\"\"Say hello.\"\"\"\n    \
# a line comment\n    \
message = \"hi\"\n    \
return message\n";
    let items = extract_items("python", source)?;

    ensure(
        kinds(&items).contains(&&EvidenceKind::Structure),
        "kinds(&items).contains(&&EvidenceKind::Structure)",
    )?;
    ensure(
        kinds(&items).contains(&&EvidenceKind::Import),
        "kinds(&items).contains(&&EvidenceKind::Import)",
    )?;
    ensure(
        kinds(&items).contains(&&EvidenceKind::Comment),
        "kinds(&items).contains(&&EvidenceKind::Comment)",
    )?;

    let structure = items
        .iter()
        .find(|item| item.kind == EvidenceKind::Structure)
        .check("a structure item")?;
    ensure_eq(
        field(&structure.payload, "name"),
        &"greet",
        "structure.payload[\"name\"] == \"greet\"",
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
        structure_keys.len() >= 2,
        "expected at least struct + impl/method",
    )?;
    ensure_eq(
        &structure_keys.len(),
        &(structure_keys
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()),
        "structure_keys.len() == structure_keys .iter() .collect::<std::collections::HashSet<_>>() .len()",
    )?;
    Ok(())
}

/// A file nested past the walk bound must come back truncated, not abort the
/// process. Without the bound the walkers would recurse once per nesting
/// level until the thread stack ran out, and a stack overflow is not a
/// catchable panic — `fatal runtime error: stack overflow, aborting` — so the
/// tool call never returns and the caller's session dies with it. 6 000 nested
/// parens are deep enough to show it.
#[test]
fn a_source_nested_past_the_walk_bound_is_truncated_not_aborted() -> TestResult {
    let nesting = usize::try_from(crate::evidence::MAX_WALK_DEPTH).check("fits")? * 4;
    let source = format!("x = {}1{}\n", "(".repeat(nesting), ")".repeat(nesting));

    let (items, depth_capped) =
        extract("python", &source).check("a deep source is truncated, not a failure")?;

    ensure(
        depth_capped,
        "a walk that stopped at the bound must report it, or the extraction \
         claims a completeness it did not reach",
    )?;
    ensure(!items.is_empty(), "the walk still yields what it reached")?;
    Ok(())
}

/// 10 000 levels of `a!(a!(...))` must not abort the process with a stack
/// overflow (as unbounded macro recursion would) nor reparse the whole body
/// once per level. The nesting bound makes it a
/// truncated extraction, reported the same way as the tree-depth bound.
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
        items
            .iter()
            .any(|item| *field(&item.payload, "callee_name") == "expect"),
        "items .iter() .any(|item| item.payload[\"callee_name\"] == \"expect\")",
    )?;
    Ok(())
}

/// A minified single-line file must not cost line-length steps per evidence
/// item (`char_column` walking back to the line start and counting). Here 20 000
/// calls sit behind a 16 MB block comment on ONE line, so a per-call count
/// would cost ~16 MB twice per call (~6e11 bytes, over a minute); with the
/// per-file `LineIndex` the whole extraction is linear.
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
    ensure(
        kinds(&items).contains(&&EvidenceKind::Diagnostic),
        "kinds(&items).contains(&&EvidenceKind::Diagnostic)",
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

// --- generic variable/field extraction (extract_bindings) ---

#[test]
fn rust_let_bindings_surface_as_variable_symbols() -> TestResult {
    let source =
        "fn pick(first: u32, second: u32) -> u32 {\n    let winner = first;\n    winner\n}\n";
    let items = extract_items("rust", source)?;

    let vars: Vec<&EvidenceItem> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol
                && field(&item.payload, "kind").as_str() == Some("Variable")
        })
        .collect();

    let names: Vec<&str> = vars
        .iter()
        .map(|item| {
            field(&item.payload, "name")
                .as_str()
                .check("name is a string")
        })
        .collect::<TestResult<_>>()?;

    // `first` and `second` are parameters; `winner` is a let binding.
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
        .filter(|item| {
            item.kind == EvidenceKind::Symbol
                && field(&item.payload, "kind").as_str() == Some("Variable")
                && item.origin == EvidenceOrigin::GenericWalker
        })
        .map(|item| {
            field(&item.payload, "name")
                .as_str()
                .check("name is a string")
        })
        .collect::<TestResult<_>>()?;

    ensure(
        names.contains(&"name"),
        format!("expected param `name`, got {names:?}"),
    )?;
    ensure(
        names.contains(&"age"),
        format!("expected param `age`, got {names:?}"),
    )?;
    Ok(())
}

#[test]
fn rust_struct_fields_surface_as_field_symbols() -> TestResult {
    let source = "struct Person {\n    name: String,\n    age: u8,\n}\n";
    let items = extract_items("rust", source)?;

    let fields: Vec<&EvidenceItem> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol
                && field(&item.payload, "kind").as_str() == Some(r#"Other("Field")"#)
        })
        .collect();

    let names: Vec<&str> = fields
        .iter()
        .map(|item| {
            field(&item.payload, "name")
                .as_str()
                .check("name is a string")
        })
        .collect::<TestResult<_>>()?;

    ensure(
        names.contains(&"name"),
        format!("expected field `name`, got {names:?}"),
    )?;
    ensure(
        names.contains(&"age"),
        format!("expected field `age`, got {names:?}"),
    )?;
    Ok(())
}

#[test]
fn binding_names_are_sliced_from_source_not_hardcoded() -> TestResult {
    // A near-duplicate pair: the extractor must report the exact text of
    // each binding so the AI can spot `file_path` vs `filepath`.
    let source = "fn index(file_path: &str, filepath: &str) {\n    let file_path_len = file_path.len();\n    let filepath_len = filepath.len();\n}\n";
    let items = extract_items("rust", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Symbol)
        .map(|item| {
            field(&item.payload, "name")
                .as_str()
                .check("name is a string")
        })
        .collect::<TestResult<_>>()?;

    ensure(names.contains(&"file_path"), format!("got {names:?}"))?;
    ensure(names.contains(&"filepath"), format!("got {names:?}"))?;
    ensure(names.contains(&"file_path_len"), format!("got {names:?}"))?;
    ensure(names.contains(&"filepath_len"), format!("got {names:?}"))?;
    Ok(())
}

#[test]
fn python_assignments_surface_as_variable_symbols() -> TestResult {
    let source = "def run():\n    file_path = 'a'\n    filepath = 'b'\n    return file_path\n";
    let items = extract_items("python", source)?;

    let names: Vec<&str> = items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol && item.origin == EvidenceOrigin::GenericWalker
        })
        .map(|item| {
            field(&item.payload, "name")
                .as_str()
                .check("name is a string")
        })
        .collect::<TestResult<_>>()?;

    ensure(
        names.contains(&"file_path") && names.contains(&"filepath"),
        format!("expected both near-duplicate python bindings, got {names:?}"),
    )?;
    Ok(())
}

#[test]
fn pack_covered_languages_never_double_report_imports_via_the_generic_walker() -> TestResult {
    // The generic import walker is gated on the pack finding zero imports
    // for a file. A python file with imports must produce exactly the
    // pack's Import items and no GenericWalker-origin duplicates.
    let source = "import os\nfrom .sibling import helper\n";
    let items = extract_items("python", source)?;
    let import_items: Vec<&EvidenceItem> = items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Import)
        .collect();
    ensure(!import_items.is_empty(), "the pack finds the imports")?;
    ensure(
        import_items
            .iter()
            .all(|item| item.origin != EvidenceOrigin::GenericWalker),
        "pack-covered languages must not re-extract imports with the generic walker",
    )?;
    Ok(())
}

#[test]
fn binding_extraction_gracefully_degrades_for_an_unsupported_language() -> TestResult {
    // `extract` itself errors on an unknown language, but the binding
    // walker is invoked only after `process` succeeds — so this guards
    // the `get_parser` fallback path indirectly through a language the
    // pack knows but whose grammar yields no `identifier` children for
    // the matched node types. Rust with a source containing no bindings
    // at all should simply return no variable/field items.
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

    let extraction =
        extract_extra_walkers("no-such-language-for-walker-test", &input, true, &mut [])
            .check("run the walkers")?;

    ensure(extraction.items.is_empty(), "extraction.items.is_empty()")?;
    ensure(!extraction.timed_out, "no deadline, so no timeout")?;
    ensure(
        extraction.truncated,
        "a failed parser setup must not read as a complete extraction",
    )?;
    Ok(())
}

/// Elements in each wide node of [`wide_node_source`]: wide enough that
/// iterating a node's children by index (`named_child(i)` restarts from the
/// first sibling each time) or climbing `parent()` per call (tree-sitter
/// descends from the root through the same sibling list) costs ~450 million
/// sibling steps per wide node and walker, while a linear walk of the ~360 KB
/// file takes well under a second.
const WIDE_NODE_ELEMENTS: usize = 30_000;

/// One function holding an array literal and a `vec!` macro, each with
/// `WIDE_NODE_ELEMENTS` calls: one wide node for the tree walkers and one
/// for the macro re-parse.
fn wide_node_source() -> String {
    let elements = "f(1), ".repeat(WIDE_NODE_ELEMENTS);
    format!("fn run() {{\n    let a = [{elements}];\n    let b = vec![{elements}];\n}}\n")
}

/// The binding, import and Rust call walkers must be linear in a node's
/// width. The deadline turns a regression into a failed assertion instead of
/// a hang: a quadratic walk outlasts it and comes back timed out.
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

    let extraction =
        extract_extra_walkers("rust", &input, true, &mut []).check("run the walkers")?;

    let elapsed = started.elapsed();
    ensure(
        !extraction.timed_out,
        format!("the walk timed out after {elapsed:?}"),
    )?;
    ensure(!extraction.truncated, "!extraction.truncated")?;
    let calls: Vec<&EvidenceItem> = extraction
        .items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Call)
        .collect();
    // Every `f(1)` in both nodes, plus the `vec!` invocation itself.
    ensure_eq(
        &calls.len(),
        &(2 * WIDE_NODE_ELEMENTS + 1),
        "calls.len() == 2 * WIDE_NODE_ELEMENTS + 1",
    )?;
    ensure(
        calls
            .iter()
            .all(|item| *field(&item.payload, "caller") == "run"),
        "calls.iter().all(|item| item.payload[\"caller\"] == \"run\")",
    )?;
    let bindings: Vec<&serde_json::Value> = extraction
        .items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Symbol)
        .map(|item| field(&item.payload, "name"))
        .collect();
    ensure_eq(&bindings, &["a", "b"], "bindings == [\"a\", \"b\"]")?;
    ensure(
        elapsed < std::time::Duration::from_secs(5),
        format!("walking {WIDE_NODE_ELEMENTS}-wide nodes took {elapsed:?}"),
    )?;
    Ok(())
}

/// A walker loop checks the deadline: once it has run out the walk stops,
/// keeps nothing it did not reach, and reports the file timed out (recorded
/// as `parse_timeout`), not truncated.
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
    let mut out = Extraction::default();
    let mut counter = 0usize;

    crate::evidence::bindings::walk_bindings(&tree.root_node(), &input, &mut out, &mut counter)
        .check("walk the bindings")?;
    crate::evidence::generic_imports::walk_imports(
        &tree.root_node(),
        &input,
        &mut out,
        &mut counter,
    )
    .check("walk the imports")?;

    ensure(out.items.is_empty(), format!("{:?}", out.items))?;
    ensure(out.timed_out, "out.timed_out")?;
    ensure(!out.truncated, "!out.truncated")?;
    Ok(())
}

#[path = "normalize_stack_tests.rs"]
mod stack;
