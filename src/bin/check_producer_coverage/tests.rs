//! Tests for the producer-coverage gate: the scanner's rules, and the gate's
//! verdict on this repository's own enums.

use super::*;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn enum_variants_come_from_the_declaration() {
    let source = "pub enum Thing {\n    Alpha,\n    Beta,\n    Gamma,\n}\n";
    assert_eq!(
        scan::enum_variants(source, "Thing"),
        Some(vec![
            "Alpha".to_owned(),
            "Beta".to_owned(),
            "Gamma".to_owned()
        ])
    );
}

#[test]
fn enum_variants_ignore_attributes_and_payloads() {
    let source = "pub enum Thing {\n    /// docs\n    Alpha,\n    Failed { reason: String },\n}\n";
    assert_eq!(
        scan::enum_variants(source, "Thing"),
        Some(vec!["Alpha".to_owned(), "Failed".to_owned()])
    );
}

#[test]
fn enum_variants_survive_multi_line_payloads() {
    let source = "pub enum Thing {\n    Alpha,\n    Failed {\n        reason: String,\n    },\n    Pair(\n        String,\n        u8,\n    ),\n    Omega,\n}\n";
    assert_eq!(
        scan::enum_variants(source, "Thing"),
        Some(vec![
            "Alpha".to_owned(),
            "Failed".to_owned(),
            "Pair".to_owned(),
            "Omega".to_owned()
        ])
    );
}

#[test]
fn a_braced_value_is_a_producer() {
    let produced =
        scan::producer_variants("let item = Thing { kind: Thing::Alpha, x: 1 };", "Thing");
    assert_eq!(produced, vec!["Alpha".to_owned()]);

    let produced = scan::producer_variants("run.completeness = Thing::Beta;", "Thing");
    assert_eq!(produced, vec!["Beta".to_owned()]);
}

#[test]
fn a_match_arm_is_not_a_producer() {
    let source = "match kind {\n    Thing::Alpha => \"Alpha\",\n    Thing::Beta => \"Beta\",\n}";
    assert!(
        scan::producer_variants(source, "Thing").is_empty(),
        "the SQL mirror's arms enumerate every variant; counting them would make the gate vacuous"
    );
}

#[test]
fn a_comparison_or_binding_pattern_is_not_a_producer() {
    assert!(scan::producer_variants("if x.kind == Thing::Alpha { }", "Thing").is_empty());
    assert!(scan::producer_variants("if let Thing::Alpha = x { }", "Thing").is_empty());
}

#[test]
fn a_path_qualified_comparison_is_not_a_producer() {
    for source in [
        "if x.kind == model::EvidenceKind::Alpha { }",
        "if x.kind != crate::model::Thing::Alpha { }",
        "assert!(kind == a::b::Thing::Alpha);",
        "if kind >= ::Thing::Alpha { }",
    ] {
        assert!(
            scan::producer_variants(source, "Thing").is_empty()
                && scan::producer_variants(source, "EvidenceKind").is_empty(),
            "a comparison is not a construction: {source}"
        );
    }
}

#[test]
fn a_path_qualified_value_is_a_producer() {
    let source = "let item = Item { kind: crate::model::Thing::Alpha };\nrun.x = a::Thing::Beta;";
    assert_eq!(
        scan::producer_variants(source, "Thing"),
        vec!["Alpha".to_owned(), "Beta".to_owned()]
    );
}

#[test]
fn a_value_in_a_block_tail_is_a_producer() {
    let source = "let kind = if flag {\n    Thing::Alpha\n} else {\n    Thing::Beta\n};";
    let mut produced = scan::producer_variants(source, "Thing");
    produced.sort();
    assert_eq!(produced, vec!["Alpha".to_owned(), "Beta".to_owned()]);
}

#[test]
fn production_code_drops_an_inline_test_module() {
    let source = "fn real() {}\n#[cfg(test)]\nmod tests {\n    fn helper() -> Thing { Thing::Alpha }\n}\nfn after() {}\n";
    let code = scan::production_code(source);
    assert!(code.contains("fn real"));
    assert!(code.contains("fn after"));
    assert!(
        !code.contains("Thing::Alpha"),
        "a cfg(test) fixture must not count as a producer: {code}"
    );
}

#[test]
fn production_code_drops_items_gated_on_test_inside_all() {
    let source = "#[cfg(all(test, unix))]\nfn fixture() -> Thing { Thing::Alpha }\nfn real() {}\n";
    let code = scan::production_code(source);
    assert!(code.contains("fn real"));
    assert!(
        !code.contains("Thing::Alpha"),
        "a cfg(all(test, ..)) fixture must not count as a producer: {code}"
    );
}

#[test]
fn production_code_keeps_items_that_exist_outside_test_builds() {
    for attribute in [
        "#[cfg(not(test))]",
        "#[cfg(any(target_os = \"macos\", test))]",
        "#[cfg(feature = \"test\")]",
        "#[cfg(unix)]",
    ] {
        let source = format!("{attribute}\nfn kept() -> Thing {{ Thing::Alpha }}\n");
        let code = scan::production_code(&source);
        assert!(
            code.contains("Thing::Alpha"),
            "{attribute} was stripped: {code}"
        );
    }
}

#[test]
fn production_code_drops_cfg_test_items() {
    let source =
        "#[cfg(test)]\nuse x::y;\n#[cfg(test)]\n#[path = \"z.rs\"]\nmod z;\nfn real() {}\n";
    let code = scan::production_code(source);
    assert!(!code.contains("use x::y"));
    assert!(!code.contains("mod z"));
    assert!(code.contains("fn real"));
}

#[test]
fn production_code_drops_a_line_comment_that_names_a_variant() {
    let source = "// pair with `kind: Thing::Alpha` in the writer\nfn real() {}\n";
    let code = scan::production_code(source);
    assert!(code.contains("fn real"));
    assert!(
        !code.contains("Thing::Alpha"),
        "a comment naming a variant must not count as a producer: {code}"
    );
}

#[test]
fn the_gate_finds_every_variant_of_this_projects_enums_produced() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let reports = check(root).check("the project's own evidence enums parse")?;
    assert_eq!(reports.len(), 2, "one report per gated enum");
    for report in &reports {
        assert!(report.declared > 0, "{} has variants", report.name);
        assert!(
            report.uncovered.is_empty(),
            "{} variants with no producer: {:?}",
            report.name,
            report.uncovered
        );
        assert_eq!(report.covered, report.declared);
    }
    Ok(())
}
