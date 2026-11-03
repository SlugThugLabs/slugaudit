use super::*;

/// The stack the stack-safety tests run extraction on: deliberately an eighth
/// of the smallest stack production extracts on (2 MiB, see
/// `crate::evidence::MAX_WALK_DEPTH`), so a regression trips here long before
/// it can abort the server.
const SMALL_TEST_STACK: usize = 256 * 1024;

/// Runs `extract(language, source)` on a thread with `SMALL_TEST_STACK`. A
/// stack overflow is not a panic: it aborts the whole test process, which is
/// exactly the failure these tests exist to catch.
fn extract_on_small_stack(
    language: &'static str,
    source: String,
) -> TestResult<(Vec<EvidenceItem>, bool)> {
    std::thread::Builder::new()
        .stack_size(SMALL_TEST_STACK)
        .spawn(move || extract(language, &source))
        .check("spawn small-stack thread")?
        .join()
        .map_err(|panic| format!("{panic:?}"))
        .check("extraction thread panicked")?
        .check("extract")
}

/// `levels` nested `m!(...)` invocations, each wrapping its body in `parens`
/// parentheses: every level reparses its body and walks a fresh synthetic tree
/// just under the per-tree bound.
fn nested_macro_source(levels: usize, parens: usize) -> String {
    let open = format!("m!({}", "(".repeat(parens));
    let close = format!("{})", ")".repeat(parens));
    format!(
        "fn run() {{ {}v.expect(){}; }}\n",
        open.repeat(levels),
        close.repeat(levels)
    )
}

/// Reproducer: 30 macro levels x 480 parens, a 29 KB file. If the macro
/// bound (64) and the tree-walk bound (512) multiplied instead of sharing one
/// budget, it would abort a release build on a 2 MiB thread; on this 256 KiB
/// stack that shows as the test process aborting with "has overflowed its
/// stack".
#[test]
fn a_rust_file_of_nested_paren_heavy_macros_is_partial_on_a_small_stack() -> TestResult {
    let (items, depth_capped) = extract_on_small_stack("rust", nested_macro_source(30, 480))?;

    ensure(depth_capped, "the cut must be reported as Partial")?;
    ensure(!items.is_empty(), "the walk still yields what it reached")?;
    Ok(())
}

#[test]
fn a_deeper_nested_macro_variant_is_partial_on_a_small_stack() -> TestResult {
    let (_, depth_capped) = extract_on_small_stack("rust", nested_macro_source(200, 505))?;

    ensure(depth_capped, "the cut must be reported as Partial")?;
    Ok(())
}

#[test]
fn deep_non_macro_nesting_is_partial_on_a_small_stack() -> TestResult {
    let nesting = 20_000;
    let rust = format!(
        "fn run() {{ let x = {}f(1){}; }}\n",
        "g(".repeat(nesting),
        ")".repeat(nesting)
    );
    let python = format!("x = {}1{}\n", "(".repeat(nesting), ")".repeat(nesting));

    let (_, rust_capped) = extract_on_small_stack("rust", rust)?;
    let (_, python_capped) = extract_on_small_stack("python", python)?;

    ensure(rust_capped, "a cut Rust call walk must be reported")?;
    ensure(python_capped, "a cut binding walk must be reported")?;
    Ok(())
}

/// Real code nests a few macros a few expressions deep; sharing one recursion
/// budget across macro re-entry must not mark such a file Partial.
#[test]
fn a_moderately_nested_real_world_rust_file_is_not_partial() -> TestResult {
    let source = r#"
mod outer {
    impl Server {
        fn handle(&self, request: Request) -> Result<Response, Error> {
            for item in request.items.iter().filter(|i| i.enabled) {
                match item.kind {
                    Kind::A => {
                        if let Some(value) = item.value.as_ref() {
                            tracing::info!(
                                value = %format!("{}", vec![value.len(), helper(compute(value.get(0)))].len()),
                                "handled"
                            );
                            assert_eq!(check(value), Ok(()), "{}", format!("{:?}", value.clone()));
                        }
                    }
                    _ => println!("{}", format!("{}", write!(out, "{}", item.name.trim()).is_ok())),
                }
            }
            Ok(Response::new(vec![json!({"a": [1, 2, {"b": nested(call(x))}]})]))
        }
    }
}
"#;

    let (items, depth_capped) = extract_on_small_stack("rust", source.to_owned())?;

    ensure(!depth_capped, "ordinary nesting must stay Full")?;
    for name in ["compute", "check", "trim", "nested"] {
        ensure(
            items
                .iter()
                .any(|item| *field(&item.payload, "callee_name") == name),
            format!("the call inside the macros, {name}, must still be recovered"),
        )?;
    }
    Ok(())
}

/// SlugAudit's own walkers use constant stack, but the language pack's JSON
/// value extraction recurses (bounded at its own depth 512) and is the deepest
/// stack user measured on the extraction path: 1646 KiB in a debug build,
/// 455 KiB in release (pack 1.18.0, Rust 1.97.1). This pins that
/// it still fits the smallest production stack (2 MiB), so a pack bump that
/// deepens it fails here instead of aborting the server.
#[test]
fn a_json_file_nested_past_the_pack_bound_fits_the_production_stack() -> TestResult {
    let nesting = 20_000;
    let source = format!("{}1{}", "[".repeat(nesting), "]".repeat(nesting));

    let (_, depth_capped) = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || extract("json", &source))
        .check("spawn production-size thread")?
        .join()
        .map_err(|panic| format!("{panic:?}"))
        .check("extraction thread panicked")?
        .check("extract")?;

    ensure(
        depth_capped,
        "returning at all is the point; the walker cut is still reported",
    )?;
    Ok(())
}

/// The pack caches one parser per language per thread, and a parse cancelled
/// by the progress callback leaves that parser to resume the cancelled
/// document on its next use (tree-sitter requires `reset()`, which the pack's
/// `process()` does not call). A bounded extraction therefore must not run
/// on a thread whose cached parser it can poison: the file parsed next on
/// the same thread would be read against the wrong source (a
/// slice-out-of-bounds panic inside the pack).
#[test]
fn a_parse_cut_at_the_deadline_does_not_corrupt_the_next_parse_on_the_same_thread() -> TestResult {
    let generous = Deadline::after(std::time::Duration::from_mins(10));
    let line = "pub fn f(x: Option<u8>) -> u8 { x.expect() }\n";
    let big = line.repeat(400 * 1024 / line.len());
    extract_within("rust", "pub fn warm() {}\n", Some(&generous))
        .check("warm the cached parser")?;

    let cut = extract_within(
        "rust",
        &big,
        Some(&Deadline::after(std::time::Duration::from_millis(20))),
    );
    let next = extract_within("rust", "pub fn after() {}\n", Some(&generous))
        .check("the next file parses normally")?;

    ensure(
        matches!(cut, Err(ExtractError::Pack(PackError::ParseTimeout { .. }))),
        "the large parse is cancelled by its 20 ms budget",
    )?;
    let names: Vec<&str> = next
        .items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Structure)
        .filter_map(|item| field(&item.payload, "name").as_str())
        .collect();
    ensure_eq(&names, &["after"], "names == [\"after\"]")?;
    ensure(!next.timed_out, "!next.timed_out")?;
    Ok(())
}
