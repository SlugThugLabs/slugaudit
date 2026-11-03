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
        "variants::enum_variants(source, \"Thing\") == Some(vec![ \"Alpha\".to_owned(), \"Beta\".to_owned(), \"Gamma\".to_owned() ])",
    )?;
    Ok(())
}

#[test]
fn enum_variants_ignore_attributes_and_payloads() -> TestResult {
    let source = "pub enum Thing {\n    /// docs\n    Alpha,\n    Failed { reason: String },\n}\n";
    ensure_eq(
        &variants::enum_variants(source, "Thing"),
        &Some(vec!["Alpha".to_owned(), "Failed".to_owned()]),
        "variants::enum_variants(source, \"Thing\") == Some(vec![\"Alpha\".to_owned(), \"Failed\".to_owned()])",
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
        "variants::enum_variants(source, \"Thing\") == Some(vec![ \"Alpha\".to_owned(), \"Failed\".to_owned(), \"Pair\".to_owned(), \"Omega\".to_owned() ])",
    )?;
    Ok(())
}

#[test]
fn a_braced_value_is_a_producer() -> TestResult {
    let produced =
        variants::producer_variants("let item = Thing { kind: Thing::Alpha, x: 1 };", "Thing");
    ensure_eq(
        &produced,
        &vec!["Alpha".to_owned()],
        "produced == vec![\"Alpha\".to_owned()]",
    )?;

    let produced = variants::producer_variants("run.completeness = Thing::Beta;", "Thing");
    ensure_eq(
        &produced,
        &vec!["Beta".to_owned()],
        "produced == vec![\"Beta\".to_owned()]",
    )?;
    Ok(())
}

#[test]
fn a_match_arm_is_not_a_producer() -> TestResult {
    let source = "match kind {\n    Thing::Alpha => \"Alpha\",\n    Thing::Beta => \"Beta\",\n}";
    ensure(
        variants::producer_variants(source, "Thing").is_empty(),
        "the SQL mirror's arms enumerate every variant; counting them would make the gate vacuous",
    )?;
    Ok(())
}

#[test]
fn a_comparison_or_binding_pattern_is_not_a_producer() -> TestResult {
    ensure(
        variants::producer_variants("if x.kind == Thing::Alpha { }", "Thing").is_empty(),
        "variants::producer_variants(\"if x.kind == Thing::Alpha { }\", \"Thing\").is_empty()",
    )?;
    ensure(
        variants::producer_variants("if let Thing::Alpha = x { }", "Thing").is_empty(),
        "variants::producer_variants(\"if let Thing::Alpha = x { }\", \"Thing\").is_empty()",
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
            format!("a comparison is not a construction: {source}"),
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
        "variants::producer_variants(source, \"Thing\") == vec![\"Alpha\".to_owned(), \"Beta\".to_owned()]",
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
        "produced == vec![\"Alpha\".to_owned(), \"Beta\".to_owned()]",
    )?;
    Ok(())
}

#[test]
fn production_code_drops_an_inline_test_module() -> TestResult {
    let source = "fn real() {}\n#[cfg(test)]\nmod tests {\n    fn helper() -> Thing { Thing::Alpha }\n}\nfn after() {}\n";
    let code = scan::production_code(source).check("strip test code")?;
    ensure(code.contains("fn real"), "code.contains(\"fn real\")")?;
    ensure(code.contains("fn after"), "code.contains(\"fn after\")")?;
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
    ensure(code.contains("fn real"), "code.contains(\"fn real\")")?;
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
    ensure(!code.contains("use x::y"), "!code.contains(\"use x::y\")")?;
    ensure(!code.contains("mod z"), "!code.contains(\"mod z\")")?;
    ensure(code.contains("fn real"), "code.contains(\"fn real\")")?;
    Ok(())
}

#[test]
fn production_code_drops_a_line_comment_that_names_a_variant() -> TestResult {
    let source = "// pair with `kind: Thing::Alpha` in the writer\nfn real() {}\n";
    let code = scan::production_code(source).check("strip test code")?;
    ensure(code.contains("fn real"), "code.contains(\"fn real\")")?;
    ensure(
        !code.contains("Thing::Alpha"),
        format!("a comment naming a variant must not count as a producer: {code}"),
    )?;
    Ok(())
}

#[test]
fn the_gate_finds_every_variant_of_this_projects_enums_produced() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let reports = check(root).check("the project's own evidence enums parse")?;
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
            "report.covered == report.declared",
        )?;
    }
    Ok(())
}
