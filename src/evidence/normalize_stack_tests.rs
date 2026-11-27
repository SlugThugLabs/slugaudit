use super::*;

const SMALL_TEST_STACK: usize = 256 * 1024;

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

fn nested_macro_source(levels: usize, parens: usize) -> String {
    let open = format!("m!({}", "(".repeat(parens));
    let close = format!("{})", ")".repeat(parens));
    format!(
        "fn run() {{ {}v.expect(){}; }}\n",
        open.repeat(levels),
        close.repeat(levels)
    )
}

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
    let calls = format!(
        "fn run() {{ let x = {}f(1){}; }}\n",
        "g(".repeat(nesting),
        ")".repeat(nesting)
    );
    let tuple_patterns = format!(
        "fn unpack() {{ let {}leaf{} = src; }}\n",
        "(".repeat(nesting),
        ",)".repeat(nesting)
    );

    let (_, calls_capped) = extract_on_small_stack("rust", calls)?;
    let (_, patterns_capped) = extract_on_small_stack("rust", tuple_patterns)?;

    ensure(calls_capped, "a cut Rust call walk must be reported")?;
    ensure(patterns_capped, "a cut Rust pattern walk must be reported")?;
    Ok(())
}

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
            items.iter().any(
                |item| matches!(&item.payload, EvidencePayload::Call(c) if c.callee_name == name),
            ),
            format!("the call inside the macros, {name}, must still be recovered"),
        )?;
    }
    Ok(())
}

#[test]
fn a_deeply_nested_rust_array_fits_the_production_stack() -> TestResult {
    let nesting = 20_000;
    let source = format!(
        "fn main() {{ let _ = {}1{}; }}\n",
        "[".repeat(nesting),
        "]".repeat(nesting)
    );

    let (_, depth_capped) = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || extract("rust", &source))
        .check("spawn production-size thread")?
        .join()
        .map_err(|panic| format!("{panic:?}"))
        .check("extraction thread panicked")?
        .check("extract")?;

    ensure(depth_capped, "walker depth cut is reported")?;
    Ok(())
}

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
        Some(&Deadline::after(std::time::Duration::from_millis(1))),
    );
    let next = extract_within("rust", "pub fn after() {}\n", Some(&generous))
        .check("the next file parses normally")?;

    ensure(
        matches!(cut, Err(ExtractError::Pack(PackError::ParseTimeout { .. })))
            || cut.as_ref().is_ok_and(|ext| ext.timed_out),
        format!("the large parse is cancelled by its 1 ms budget: {cut:?}"),
    )?;
    let names: Vec<&str> = next
        .items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Structure(s) => s.name.as_deref(),
            _ => None,
        })
        .collect();
    ensure_eq(&names, &["after"], "names == [\"after\"]")?;
    ensure(!next.timed_out, "!next.timed_out")?;
    Ok(())
}
