//! C7 grammar-matrix validation: `is_field_declaration` matches node
//! kinds *by exact name*. These tests pin the fixed grammar matrix the
//! audit (C7) called for: for every language, a class/struct with known
//! members must surface exactly those members as Field symbols — and,
//! crucially, must NOT emit a duplicate from a container node (the
//! `field_declaration_list` false positive the old substring match
//! produced). The sources are deliberately minimal so the expected member
//! set is unambiguous.
//!
//! Split from `normalize_tests.rs` (2026-08-12, C7) to keep each file
//! under the source-size cap; the `#[path]` include in `normalize.rs`
//! wires this in as a sibling test module.

use super::*;

// --- C7 grammar-matrix validation ---
//
// `is_field_declaration` matches node kinds *by exact name*. These tests
// pin the fixed grammar matrix the audit (C7) called for: for every
// language, a class/struct with known members must surface exactly those
// members as Field symbols — and, crucially, must NOT emit a duplicate
// from a container node (the `field_declaration_list` false positive the
// old substring match produced). The sources are deliberately minimal so
// the expected member set is unambiguous.

fn field_names(items: &[EvidenceItem]) -> Vec<&str> {
    items
        .iter()
        .filter(|item| {
            item.kind == EvidenceKind::Symbol
                && item.payload["kind"].as_str() == Some(r#"Other("Field")"#)
        })
        .filter_map(|item| item.payload["name"].as_str())
        .collect()
}

#[test]
fn c7_field_declaration_kinds_surface_exactly_the_members_in_rust() {
    let source = "struct S { a: i32, b: String }\n";
    let items = extract("rust", source).expect("process rust");
    let names = field_names(&items);
    assert_eq!(
        names,
        vec!["a", "b"],
        "exact members, no container duplicate"
    );
}

#[test]
fn c7_field_declaration_kinds_surface_exactly_the_members_in_go_and_c_family() {
    for (language, source) in [
        ("go", "type S struct { a int; b string }\n"),
        ("c", "struct S { int a; int b; };\n"),
        ("cpp", "struct S { int a; int b; };\n"),
        ("java", "class C { int a; String b; }\n"),
        ("csharp", "class C { public int a; private string b; }\n"),
        ("ocaml", "type t = { a : int; b : string }\n"),
    ] {
        let items =
            extract(language, source).unwrap_or_else(|e| panic!("{language}: process failed: {e}"));
        let names = field_names(&items);
        assert_eq!(
            names,
            vec!["a", "b"],
            "{language}: exact members, no duplicate"
        );
    }
}

#[test]
fn c7_property_declaration_kinds_surface_exactly_the_members_in_swift_and_kotlin() {
    for (language, source) in [
        ("swift", "struct S { let a: Int; var b: String }\n"),
        ("kotlin", "class C { val a: Int; var b: String }\n"),
    ] {
        let items =
            extract(language, source).unwrap_or_else(|e| panic!("{language}: process failed: {e}"));
        let names = field_names(&items);
        assert_eq!(
            names,
            vec!["a", "b"],
            "{language}: exact members, no duplicate"
        );
    }
}

#[test]
fn c7_field_definition_kinds_surface_exactly_the_members_in_javascript_and_typescript() {
    for (language, source) in [
        ("javascript", "class C { a = 1; b = 2; }\n"),
        (
            "typescript",
            "class C { a: number = 1; b: string = 'x'; }\n",
        ),
    ] {
        let items =
            extract(language, source).unwrap_or_else(|e| panic!("{language}: process failed: {e}"));
        let names = field_names(&items);
        assert_eq!(
            names,
            vec!["a", "b"],
            "{language}: exact members, no duplicate"
        );
    }
}

#[test]
fn c7_languages_without_field_declaration_kinds_produce_no_field_symbols() {
    // Languages whose grammars have no member-declaration node kind must
    // produce no Field symbols at all — the exact-match list must not
    // fire on their ordinary expression/assignment kinds (the audit's
    // "no false positives per language" requirement). Python attributes
    // are assignments (Variable symbols), not Field symbols.
    for (language, source) in [
        ("python", "class C:\n    a = 1\n"),
        ("ruby", "class C\n  def a\n    1\n  end\nend\n"),
        ("elixir", "defmodule C do\n  def a do\n    1\n  end\nend\n"),
        ("julia", "struct S\n  a::Int\nend\n"),
        ("perl", "my $a = 1;\n"),
    ] {
        let items =
            extract(language, source).unwrap_or_else(|e| panic!("{language}: process failed: {e}"));
        assert_eq!(
            field_names(&items),
            Vec::<&str>::new(),
            "{language}: no field kinds in this grammar, so no Field symbols"
        );
    }
}

#[test]
fn c7_plain_function_source_produces_no_field_symbols_in_any_matrix_language() {
    // A function definition (no struct/class members anywhere) must never
    // produce a Field symbol in any matrix language — guards against the
    // list growing a kind that collides with function/expression nodes.
    for (language, source) in [
        ("rust", "fn f() { let x = 1; x }\n"),
        ("python", "def f():\n    return 1\n"),
        ("javascript", "function f() { return 1; }\n"),
        ("typescript", "function f(): number { return 1; }\n"),
        ("go", "func f() int { return 1 }\n"),
        ("c", "int f(void) { return 1; }\n"),
        ("cpp", "int f() { return 1; }\n"),
        ("swift", "func f() -> Int { return 1 }\n"),
        ("kotlin", "fun f(): Int = 1\n"),
        ("csharp", "int F() { return 1; }\n"),
        ("dart", "int f() => 1;\n"),
        ("julia", "f() = 1\n"),
        ("php", "<?php function f() { return 1; }\n"),
        ("perl", "sub f { return 1 }\n"),
        ("ocaml", "let f () = 1\n"),
        ("elixir", "defmodule M do\n  def f do\n    1\n  end\nend\n"),
        ("ruby", "def f\n  1\nend\n"),
        ("java", "int f() { return 1; }\n"),
    ] {
        let items =
            extract(language, source).unwrap_or_else(|e| panic!("{language}: process failed: {e}"));
        assert_eq!(
            field_names(&items),
            Vec::<&str>::new(),
            "{language}: a function with no members must produce no Field symbols"
        );
    }
}
