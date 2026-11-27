use super::{Parsed, parse_within};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::cell::Cell;
use tree_sitter::Parser;

fn rust_parser() -> TestResult<Parser> {
    let grammar = tree_sitter_language_pack::get_language("rust").check("load bundled rust")?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .check("configure parser for rust")?;
    Ok(parser)
}

#[test]
fn parse_within_returns_tree_when_budget_is_unspent() -> TestResult {
    let mut parser = rust_parser()?;
    let callbacks = Cell::new(0usize);

    let outcome = parse_within(&mut parser, "pub fn ok() -> u8 { 1 }\n", || {
        // KEEP saturating: test callback tally never approaches usize::MAX
        callbacks.set(callbacks.get().saturating_add(1));
        false
    });

    let Parsed::Tree(tree) = outcome else {
        return Err("expected Parsed::Tree when budget is unspent").check("parse_within outcome");
    };
    ensure(!tree.root_node().has_error(), "parsed tree has no errors")?;
    ensure_eq(
        &tree.root_node().kind(),
        &"source_file",
        "root node is source_file",
    )?;
    Ok(())
}

#[test]
fn parse_within_cancels_when_budget_is_spent_and_resets_parser_for_reuse() -> TestResult {
    let mut parser = rust_parser()?;
    let big = "pub fn item(v: Option<u8>) -> Option<u8> { v }\n".repeat(4096);

    let cancelled = parse_within(&mut parser, &big, || true);
    ensure(
        matches!(cancelled, Parsed::Cancelled),
        "spent callback cancels the in-flight parse",
    )?;

    let reused = parse_within(&mut parser, "pub struct Clean;\n", || false);
    let Parsed::Tree(tree) = reused else {
        return Err("expected reset parser to produce Parsed::Tree on reuse")
            .check("reused parser outcome");
    };
    ensure(
        !tree.root_node().has_error(),
        "reused parser produces an error-free tree after reset",
    )?;
    let child = tree
        .root_node()
        .named_child(0)
        .check("struct item present")?;
    ensure_eq(&child.kind(), &"struct_item", "reused parse sees new input")?;
    Ok(())
}

#[test]
fn parse_within_returns_no_tree_when_parser_has_no_language() -> TestResult {
    let mut parser = Parser::new();
    let outcome = parse_within(&mut parser, "fn orphan() {}\n", || false);
    ensure(
        matches!(outcome, Parsed::NoTree),
        "unconfigured parser yields Parsed::NoTree rather than Cancelled",
    )?;
    Ok(())
}
