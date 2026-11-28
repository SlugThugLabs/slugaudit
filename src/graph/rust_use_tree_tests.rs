use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

fn target(specifier: &str) -> String {
    specifier.to_owned()
}

#[test]
fn grouped_crate_use_expands_all_leaves() -> TestResult {
    let targets = parse_rust_import("use crate::{a, b};").check("modeled")?;
    ensure_eq(
        &targets,
        &vec![target("crate::a"), target("crate::b")],
        "grouped use must expand both crate::a and crate::b",
    )?;
    Ok(())
}

#[test]
fn nested_use_with_self_and_alias_expands_in_order() -> TestResult {
    let targets =
        parse_rust_import("use crate::foo::{self, bar, baz::qux as q};").check("modeled")?;
    ensure_eq(
        &targets,
        &vec![
            target("crate::foo"),
            target("crate::foo::bar"),
            target("crate::foo::baz::qux"),
        ],
        "self resolves to enclosing prefix and alias is stripped",
    )?;
    Ok(())
}

#[test]
fn glob_and_braced_glob_expand_identically() -> TestResult {
    let direct = parse_rust_import("use crate::foo::*;").check("direct glob")?;
    let braced = parse_rust_import("use crate::foo::{*};").check("braced glob")?;
    let expected = vec![target("crate::foo::*")];
    ensure_eq(&direct, &expected, "direct glob matches")?;
    ensure_eq(&braced, &expected, "braced glob matches")?;
    Ok(())
}

#[test]
fn root_brace_group_handles_empty_prefix_and_self() -> TestResult {
    let targets = parse_rust_import("use {crate::a, super::b};").check("root group")?;
    ensure_eq(
        &targets,
        &vec![target("crate::a"), target("super::b")],
        "root brace group expands each branch without a leading ::",
    )?;

    let only_self = parse_rust_import("use {self};").check("self group")?;
    ensure_eq(&only_self, &vec![target("self")], "use {self}; emits self")?;
    Ok(())
}

#[test]
fn empty_braces_are_modeled_with_zero_targets() -> TestResult {
    let targets = parse_rust_import("use crate::foo::{};").check("empty braces modeled")?;
    ensure(targets.is_empty(), "empty brace group emits 0 targets")?;
    Ok(())
}

#[test]
fn comments_trailing_commas_aliases_and_dedup_work_together() -> TestResult {
    let raw = "pub/*c1*/(in crate::x) /*c2*/ use crate::foo::{\n\
               // line comment\n\
               self as foo_mod,\n\
               /* outer /* nested */ */ a as _,\n\
               b::{c, d as alias_d},\n\
               a,\n\
               };";
    let targets = parse_rust_import(raw).check("commented use modeled")?;
    ensure_eq(
        &targets,
        &vec![
            target("crate::foo"),
            target("crate::foo::a"),
            target("crate::foo::b::c"),
            target("crate::foo::b::d"),
        ],
        "deduplicates identical targets in source order after stripping comments and aliases",
    )?;
    Ok(())
}

#[test]
fn malformed_use_and_empty_inputs_are_unmodeled() -> TestResult {
    for raw in [
        "",
        "pubfn use x;",
        "use;",
        "extern crate;",
        "extern crate alloc",
        "use crate::foo::{a;",
        "use crate::foo::{a} extra;",
    ] {
        ensure(
            parse_rust_import(raw).is_none(),
            format!("{raw:?} must be unmodeled"),
        )?;
    }
    Ok(())
}

#[test]
fn extern_crate_declarations_are_modeled() -> TestResult {
    ensure_eq(
        &parse_rust_import("extern crate alloc;").check("extern crate alloc")?,
        &vec![target("alloc")],
        "extern crate alloc; emits crate name",
    )?;
    ensure_eq(
        &parse_rust_import("extern crate my_crate as mc;").check("extern crate alias")?,
        &vec![target("my_crate")],
        "extern crate my_crate as mc; emits original crate name",
    )?;
    ensure_eq(
        &parse_rust_import("pub extern crate core;").check("pub extern crate")?,
        &vec![target("core")],
        "pub extern crate core; emits crate name",
    )?;
    ensure_eq(
        &parse_rust_import("#[macro_use] extern crate log;").check("attr extern crate")?,
        &vec![target("log")],
        "attributed extern crate emits crate name",
    )?;
    let self_alias =
        parse_rust_import("extern crate self as my_self;").check("extern crate self")?;
    ensure(
        self_alias.is_empty(),
        "extern crate self as my_self; is modeled with zero targets",
    )?;
    Ok(())
}

#[test]
fn deeply_nested_use_tree_beyond_bound_fails_cleanly() -> TestResult {
    let open = "{".repeat(34);
    let close = "}".repeat(34);
    let raw = format!("use crate::{open}a{close};");
    ensure(
        parse_rust_import(&raw).is_none(),
        "brace nesting deeper than the maximum limit is rejected",
    )?;
    Ok(())
}

#[test]
fn segment_chain_respects_bound_without_stack_overflow() -> TestResult {
    const OVER_SEGMENT_LIMIT: usize = MAX_PATH_SEGMENTS + 1;
    let at_limit = format!("use {};", vec!["a"; MAX_PATH_SEGMENTS].join("::"));
    let parsed = parse_rust_import(&at_limit).check("path at segment limit is modeled")?;
    ensure_eq(&parsed.len(), &1, "path at segment limit yields one target")?;

    let over_limit = format!("use {};", vec!["a"; OVER_SEGMENT_LIMIT].join("::"));
    ensure(
        parse_rust_import(&over_limit).is_none(),
        "path exceeding segment limit is unmodeled",
    )?;

    let deep_chain = format!("use {};", vec!["a"; 4096].join("::"));
    ensure(
        parse_rust_import(&deep_chain).is_none(),
        "very long :: chain terminates iteratively without stack overflow",
    )?;
    Ok(())
}

#[test]
fn expanded_target_count_respects_bound_and_deduplicates() -> TestResult {
    let items_at_limit: Vec<String> = (0..MAX_EXPANDED_TARGETS).map(|i| format!("t{i}")).collect();
    let at_limit = format!("use crate::{{{}}};", items_at_limit.join(", "));
    let parsed = parse_rust_import(&at_limit).check("use at target limit is modeled")?;
    ensure_eq(
        &parsed.len(),
        &MAX_EXPANDED_TARGETS,
        "use at target limit yields all targets",
    )?;

    let over_limit = format!("use crate::{{{}, extra}};", items_at_limit.join(", "));
    ensure(
        parse_rust_import(&over_limit).is_none(),
        "use exceeding expanded target limit is unmodeled",
    )?;

    let repeated = format!("use crate::{{{}}};", vec!["dup"; 512].join(", "));
    let deduped = parse_rust_import(&repeated).check("duplicate targets stay modeled")?;
    ensure_eq(
        &deduped,
        &vec![target("crate::dup")],
        "duplicate targets deduplicate to one entry",
    )?;
    Ok(())
}

#[test]
fn leading_double_colon_is_preserved_for_external_crate_paths() -> TestResult {
    ensure_eq(
        &parse_rust_import("use ::serde::Serialize;").check("global path")?,
        &vec![target("::serde::Serialize")],
        "single global path preserves leading ::",
    )?;
    ensure_eq(
        &parse_rust_import("use ::{a, b};").check("global group")?,
        &vec![target("::a"), target("::b")],
        "global brace group distributes leading :: to each leaf",
    )?;
    ensure_eq(
        &parse_rust_import("use {::a, b};").check("mixed root group")?,
        &vec![target("::a"), target("b")],
        "mixed root brace group preserves :: only on qualified branch",
    )?;
    ensure_eq(
        &parse_rust_import("use ::serde::{self, Serialize};").check("global self group")?,
        &vec![target("::serde"), target("::serde::Serialize")],
        "global group with self preserves leading ::",
    )?;
    ensure(
        parse_rust_import("use ::*;").is_none(),
        "bare global glob use ::*; is unmodeled",
    )?;
    Ok(())
}

#[test]
fn raw_identifiers_and_nested_wildcards_expand_cleanly() -> TestResult {
    let targets =
        parse_rust_import("use crate::r#type::{self, r#match as m, de::*};").check("raw idents")?;
    ensure_eq(
        &targets,
        &vec![
            target("crate::r#type"),
            target("crate::r#type::r#match"),
            target("crate::r#type::de::*"),
        ],
        "r# raw identifiers and nested wildcards expand in source order",
    )?;
    Ok(())
}
