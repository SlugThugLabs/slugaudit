//! Tests for the producer-coverage gate: the scanner's rules, and the gate's
//! verdict on this repository's own enums.

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn enum_variants_come_from_the_declaration() -> TestResult {
    let source = "pub enum Thing {\n    Alpha,\n    Beta,\n    Gamma,\n}\n";
    ensure_eq(
        &variants::enum_variants(source, "Thing"),
        &Some(vec![
            "Alpha".to_owned(),
            "Beta".to_owned(),
            "Gamma".to_owned(),
        ]),
        "enum_variants extracts all three declared unit variants",
    )?;
    Ok(())
}

#[test]
fn enum_variants_ignore_attributes_comments_and_payloads() -> TestResult {
    let source = "pub enum Thing {\n    /// docs\n    /* block comment Variant */\n    Alpha,\n    Failed { reason: String },\n}\n";
    ensure_eq(
        &variants::enum_variants(source, "Thing"),
        &Some(vec!["Alpha".to_owned(), "Failed".to_owned()]),
        "enum_variants ignores doc comments, block comments, and struct payloads",
    )?;
    Ok(())
}

#[test]
fn enum_variants_survive_multi_line_payloads() -> TestResult {
    let source = "pub enum Thing {\n    Alpha,\n    Failed {\n        reason: String,\n    },\n    Pair(\n        String,\n        u8,\n    ),\n    Omega,\n}\n";
    ensure_eq(
        &variants::enum_variants(source, "Thing"),
        &Some(vec![
            "Alpha".to_owned(),
            "Failed".to_owned(),
            "Pair".to_owned(),
            "Omega".to_owned(),
        ]),
        "enum_variants handles multi-line struct and tuple variant payloads",
    )?;
    Ok(())
}

#[test]
fn a_braced_or_assigned_value_is_a_producer() -> TestResult {
    let produced =
        variants::producer_variants("let item = Thing { kind: Thing::Alpha, x: 1 };", "Thing");
    ensure_eq(
        &produced,
        &vec!["Alpha".to_owned()],
        "struct-literal field construction counts as a producer",
    )?;

    let produced = variants::producer_variants("run.completeness = Thing::Beta;", "Thing");
    ensure_eq(
        &produced,
        &vec!["Beta".to_owned()],
        "assignment RHS construction counts as a producer",
    )?;
    Ok(())
}

#[test]
fn function_call_arguments_are_producers() -> TestResult {
    let source = concat!(
        "emit(Thing::Alpha);\n",
        "record(ctx, crate::model::Thing::Beta, 42);\n",
        "let wrapped = Some(Thing::Gamma);\n",
    );
    ensure_eq(
        &variants::producer_variants(source, "Thing"),
        &vec!["Alpha".to_owned(), "Beta".to_owned(), "Gamma".to_owned()],
        "leading and comma-separated function-call arguments count as producers",
    )?;
    Ok(())
}

#[test]
fn match_arms_matches_macros_and_destructuring_patterns_are_not_producers() -> TestResult {
    let source = concat!(
        "match kind {\n",
        "    Thing::Alpha => \"Alpha\",\n",
        "    (Thing::Beta, Thing::Gamma) => \"Pair\",\n",
        "    Item { kind: Thing::Delta } => \"Struct\",\n",
        "}\n",
        "let is_alpha = matches!(self.kind, Thing::Alpha);\n",
        "if let Some(Thing::Beta) = maybe_kind {}\n",
        "let (Thing::Gamma, _) = pair;\n",
    );
    ensure(
        variants::producer_variants(source, "Thing").is_empty(),
        "match arms, matches! macros, and let destructuring patterns must not count as producers",
    )?;
    Ok(())
}

#[test]
fn a_comparison_or_binding_pattern_is_not_a_producer() -> TestResult {
    ensure(
        variants::producer_variants("if x.kind == Thing::Alpha { }", "Thing").is_empty(),
        "equality comparison must not count as a producer",
    )?;
    ensure(
        variants::producer_variants("if let Thing::Alpha = x { }", "Thing").is_empty(),
        "if-let pattern binding must not count as a producer",
    )?;
    Ok(())
}

#[test]
fn a_path_qualified_comparison_is_not_a_producer() -> TestResult {
    for source in [
        "if x.kind == model::EvidenceKind::Alpha { }",
        "if x.kind != crate::model::Thing::Alpha { }",
        "assert!(kind == a::b::Thing::Alpha);",
        "if kind >= ::Thing::Alpha { }",
    ] {
        ensure(
            variants::producer_variants(source, "Thing").is_empty()
                && variants::producer_variants(source, "EvidenceKind").is_empty(),
            format!("a path-qualified comparison is not a construction: {source}"),
        )?;
    }
    Ok(())
}

#[test]
fn a_path_qualified_value_is_a_producer() -> TestResult {
    let source = "let item = Item { kind: crate::model::Thing::Alpha };\nrun.x = a::Thing::Beta;";
    ensure_eq(
        &variants::producer_variants(source, "Thing"),
        &vec!["Alpha".to_owned(), "Beta".to_owned()],
        "path-qualified struct-field and assignment values count as producers",
    )?;
    Ok(())
}

#[test]
fn a_value_in_a_block_tail_is_a_producer() -> TestResult {
    let source = "let kind = if flag {\n    Thing::Alpha\n} else {\n    Thing::Beta\n};";
    let mut produced = variants::producer_variants(source, "Thing");
    produced.sort();
    ensure_eq(
        &produced,
        &vec!["Alpha".to_owned(), "Beta".to_owned()],
        "block-tail values in conditional branches count as producers",
    )?;
    Ok(())
}

#[test]
fn production_code_drops_an_inline_test_module() -> TestResult {
    let source = "fn real() {}\n#[cfg(test)]\nmod tests {\n    fn helper() -> Thing { Thing::Alpha }\n}\nfn after() {}\n";
    let code = scan::production_code(source).check("strip test code")?;
    ensure(
        code.contains("fn real"),
        "production fn before test mod is kept",
    )?;
    ensure(
        code.contains("fn after"),
        "production fn after test mod is kept",
    )?;
    ensure(
        !code.contains("Thing::Alpha"),
        format!("a cfg(test) fixture must not count as a producer: {code}"),
    )?;
    Ok(())
}

#[test]
fn production_code_drops_items_gated_on_test_inside_all() -> TestResult {
    let source = "#[cfg(all(test, unix))]\nfn fixture() -> Thing { Thing::Alpha }\nfn real() {}\n";
    let code = scan::production_code(source).check("strip test code")?;
    ensure(code.contains("fn real"), "production fn is kept")?;
    ensure(
        !code.contains("Thing::Alpha"),
        format!("a cfg(all(test, ..)) fixture must not count as a producer: {code}"),
    )?;
    Ok(())
}

#[test]
fn production_code_keeps_items_that_exist_outside_test_builds() -> TestResult {
    for attribute in [
        "#[cfg(not(test))]",
        "#[cfg(any(target_os = \"macos\", test))]",
        "#[cfg(feature = \"test\")]",
        "#[cfg(unix)]",
    ] {
        let source = format!("{attribute}\nfn kept() -> Thing {{ Thing::Alpha }}\n");
        let code = scan::production_code(&source).check("strip test code")?;
        ensure(
            code.contains("Thing::Alpha"),
            format!("{attribute} was stripped: {code}"),
        )?;
    }
    Ok(())
}

#[test]
fn production_code_drops_cfg_test_items() -> TestResult {
    let source =
        "#[cfg(test)]\nuse x::y;\n#[cfg(test)]\n#[path = \"z.rs\"]\nmod z;\nfn real() {}\n";
    let code = scan::production_code(source).check("strip test code")?;
    ensure(!code.contains("use x::y"), "cfg(test) use item is dropped")?;
    ensure(!code.contains("mod z"), "cfg(test) mod item is dropped")?;
    ensure(code.contains("fn real"), "production fn is kept")?;
    Ok(())
}

#[test]
fn production_code_drops_cfg_test_struct_field_and_keeps_following_code() -> TestResult {
    let source = concat!(
        "struct Holder {\n",
        "    #[cfg(test)]\n",
        "    hooks: TestHooks,\n",
        "    real_field: i32,\n",
        "}\n",
        "#[cfg(test)]\n",
        "enum TestOnly {\n",
        "    Spoof,\n",
        "}\n",
    );
    let code = scan::production_code(source).check("strip cfg(test) struct field")?;
    ensure(
        !code.contains("hooks"),
        format!("cfg(test) struct field must be dropped: {code}"),
    )?;
    ensure(
        code.contains("real_field: i32"),
        format!("real struct field after cfg(test) field must be kept: {code}"),
    )?;
    ensure(
        !code.contains("TestOnly") && !code.contains("Spoof"),
        format!("subsequent cfg(test) enum must be dropped: {code}"),
    )?;
    Ok(())
}

#[test]
fn production_code_does_not_stop_cfg_test_fn_early_on_multiline_param_comma() -> TestResult {
    let source = concat!(
        "#[cfg(test)]\n",
        "fn helper(\n",
        "    first: i32,\n",
        "    second: Thing,\n",
        ") {\n",
        "    let _ = first;\n",
        "    emit(Thing::Alpha);\n",
        "}\n",
        "fn real() {\n",
        "    emit(Thing::Beta);\n",
        "}\n",
    );
    let code = scan::production_code(source).check("strip multiline cfg(test) fn")?;
    ensure(
        !code.contains("helper") && !code.contains("second:") && !code.contains("Thing::Alpha"),
        format!("entire cfg(test) fn must be dropped: {code}"),
    )?;
    ensure(
        code.contains("fn real"),
        format!("production fn after cfg(test) fn must be kept: {code}"),
    )?;
    ensure_eq(
        &variants::producer_variants(&code, "Thing"),
        &vec!["Beta".to_owned()],
        "only production producer survives",
    )?;
    Ok(())
}

#[test]
fn production_code_drops_final_cfg_test_struct_field_without_trailing_comma() -> TestResult {
    let literal_source = concat!(
        "let item = Holder {\n",
        "    real_field: Thing::Alpha,\n",
        "    #[cfg(test)]\n",
        "    test_field: Thing::Beta\n",
        "};\n",
        "fn after() {\n",
        "    emit(Thing::Gamma);\n",
        "}\n",
    );
    let literal_code =
        scan::production_code(literal_source).check("strip final cfg(test) literal field")?;
    ensure(
        literal_code.contains("real_field: Thing::Alpha") && literal_code.contains("};"),
        format!("enclosing literal structure must stay intact: {literal_code}"),
    )?;
    ensure(
        !literal_code.contains("test_field") && !literal_code.contains("Thing::Beta"),
        format!("final cfg(test) literal field must be dropped: {literal_code}"),
    )?;
    ensure_eq(
        &variants::producer_variants(&literal_code, "Thing"),
        &vec!["Alpha".to_owned(), "Gamma".to_owned()],
        "real field and following production code stay producers",
    )?;

    let struct_source = concat!(
        "struct Def {\n",
        "    kept_field: u8,\n",
        "    #[cfg(test)]\n",
        "    dropped_field: u8\n",
        "}\n",
        "fn kept_fn() {}\n",
    );
    let struct_code =
        scan::production_code(struct_source).check("strip final cfg(test) struct field")?;
    ensure(
        struct_code.contains("kept_field: u8") && struct_code.contains('}'),
        format!("enclosing struct definition must stay intact: {struct_code}"),
    )?;
    ensure(
        !struct_code.contains("dropped_field"),
        format!("final cfg(test) struct field must be dropped: {struct_code}"),
    )?;
    ensure(
        struct_code.contains("fn kept_fn"),
        format!("production fn after struct must be kept: {struct_code}"),
    )?;
    Ok(())
}

#[test]
fn production_code_strips_line_comments_block_comments_and_string_literals() -> TestResult {
    let source = concat!(
        "// pair with `kind: Thing::Alpha` in the writer\n",
        "/* outer /* nested kind: Thing::Beta */ still comment = Thing::Gamma */\n",
        "const S: &str = \"kind: Thing::Delta\";\n",
        "const ESC: &str = \"escaped \\\" kind: Thing::Epsilon\";\n",
        "const RAW: &str = r#\"raw { Thing::Zeta } and \"quotes\"\"#;\n",
        "fn real<'a>(x: &'a str) -> Thing { emit(Thing::Eta) }\n",
    );
    let code = scan::production_code(source).check("strip non-code and test items")?;
    for spoofed in ["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta"] {
        ensure(
            !code.contains(spoofed),
            format!("non-code {spoofed} survived stripping: {code}"),
        )?;
    }
    ensure_eq(
        &variants::producer_variants(&code, "Thing"),
        &vec!["Eta".to_owned()],
        "only real code construction after comments, strings, and lifetimes counts",
    )?;
    Ok(())
}

#[test]
fn the_gate_finds_every_variant_of_this_projects_enums_produced() -> TestResult {
    // The gate checks the product at the repository root, two levels above
    // this crate.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reports = check(&root).check("the project's own evidence enums parse")?;
    ensure_eq(&reports.len(), &2, "one report per gated enum")?;
    for report in &reports {
        ensure(report.declared > 0, format!("{} has variants", report.name))?;
        ensure(
            report.uncovered.is_empty(),
            format!(
                "{} variants with no producer: {:?}",
                report.name, report.uncovered
            ),
        )?;
        ensure_eq(
            &report.covered,
            &report.declared,
            "every declared variant has a production producer",
        )?;
    }
    Ok(())
}
