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
    let raw = "pub/*c1*/(in crate::x) /*c2*/ use ::crate::foo::{\n\
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
fn extern_crate_and_malformed_use_are_unmodeled() -> TestResult {
    for raw in [
        "",
        "extern crate alloc;",
        "pubfn use x;",
        "use;",
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
