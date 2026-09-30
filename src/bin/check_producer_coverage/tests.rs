//! Tests for the producer-coverage gate: the scanner's rules, and the gate's
//! verdict on this repository's own enums.

use super::*;

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
fn the_gate_finds_every_variant_of_this_projects_enums_produced() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let reports = check(root).expect("the project's own evidence enums parse");
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
}
