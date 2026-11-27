use super::*;
use crate::graph::test_support::paths;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn every_source_produces_exactly_one_edge_in_order() -> TestResult {
    let known = paths(&["pkg/a.py", "pkg/bar.py"]);
    let sources = vec![
        "from .bar import baz".to_owned(),
        "import os".to_owned(),
        "from .missing import x".to_owned(),
    ];
    let edges = resolve_imports("python", "pkg/a.py", &sources, &known);

    ensure_eq(&edges.len(), &3, "edges.len() == 3")?;
    ensure_eq(
        &edges.first().check("edges entry 0")?.resolution_kind,
        &"Resolved",
        "edges[0].resolution_kind == \"Resolved\"",
    )?;
    ensure_eq(
        &edges
            .first()
            .check("edges entry 0")?
            .to_relative_path
            .as_deref(),
        &Some("pkg/bar.py"),
        "edges[0].to_relative_path.as_deref() == Some(\"pkg/bar.py\")",
    )?;
    ensure_eq(
        &edges.first().check("edges entry 0")?.raw_import_text,
        &"from .bar import baz",
        "edges[0].raw_import_text == \"from .bar import baz\"",
    )?;

    ensure_eq(
        &edges.get(1).check("edges entry 1")?.resolution_kind,
        &"External",
        "edges[1].resolution_kind == \"External\"",
    )?;
    ensure_eq(
        &edges.get(1).check("edges entry 1")?.to_relative_path,
        &None,
        "edges[1].to_relative_path == None",
    )?;

    ensure_eq(
        &edges.get(2).check("edges entry 2")?.resolution_kind,
        &"Unresolved",
        "edges[2].resolution_kind == \"Unresolved\"",
    )?;
    ensure_eq(
        &edges.get(2).check("edges entry 2")?.to_relative_path,
        &None,
        "edges[2].to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn an_unparseable_raw_statement_becomes_unresolved_not_dropped() -> TestResult {
    let known = paths(&["a.go"]);
    let edges = resolve_imports("go", "a.go", &["import \"fmt\"".to_owned()], &known);
    ensure_eq(&edges.len(), &1, "edges.len() == 1")?;
    ensure_eq(
        &edges.first().check("edges entry 0")?.resolution_kind,
        &"Unresolved",
        "edges[0].resolution_kind == \"Unresolved\"",
    )?;
    ensure_eq(
        &edges.first().check("edges entry 0")?.raw_import_text,
        &"import \"fmt\"",
        "edges[0].raw_import_text == \"import \\\"fmt\\\"\"",
    )?;
    Ok(())
}

#[test]
fn circular_imports_are_just_two_independent_edges() -> TestResult {
    // a.py imports b, b.py imports a — nothing about resolution needs to
    // special-case a cycle; each direction resolves independently, and
    // detecting/traversing the cycle is a `query` concern (recursive CTE
    // over dependency_edges), not something the resolver prevents or flags.
    let known = paths(&["a.py", "b.py"]);
    let a_to_b = resolve_imports("python", "a.py", &["from . import b".to_owned()], &known);
    let b_to_a = resolve_imports("python", "b.py", &["from . import a".to_owned()], &known);

    ensure_eq(
        &a_to_b.first().check("a_to_b entry 0")?.resolution_kind,
        &"Unresolved",
        "a_to_b[0].resolution_kind == \"Unresolved\"",
    )?;
    ensure_eq(
        &b_to_a.first().check("b_to_a entry 0")?.resolution_kind,
        &"Unresolved",
        "b_to_a[0].resolution_kind == \"Unresolved\"",
    )?;
    // `from . import b` resolves the *package* (`.` → __init__.py), not the
    // submodule named after the imported symbol — a known limitation of
    // not modeling `from X import Y` symbol-vs-submodule ambiguity; both
    // directions are still independently and consistently classified.
    Ok(())
}

#[test]
fn an_aliased_import_resolves_on_the_module_not_the_alias() -> TestResult {
    let known = paths(&["pkg/a.py", "pkg/real_name.py"]);
    let edges = resolve_imports(
        "python",
        "pkg/a.py",
        &["from .real_name import Thing".to_owned()],
        &known,
    );
    // The alias ("Thing") never appears in `source` at all — the pack's
    // `source` field is the whole statement, and our extractor only reads
    // the module path out of it — so resolution naturally targets
    // `real_name.py`, completely independent of what name it's imported as.
    ensure_eq(
        &edges.first().check("edges entry 0")?.resolution_kind,
        &"Resolved",
        "edges[0].resolution_kind == \"Resolved\"",
    )?;
    ensure_eq(
        &edges
            .first()
            .check("edges entry 0")?
            .to_relative_path
            .as_deref(),
        &Some("pkg/real_name.py"),
        "edges[0].to_relative_path.as_deref() == Some(\"pkg/real_name.py\")",
    )?;
    Ok(())
}

#[test]
fn grouped_rust_use_expands_distinct_files_and_deduplicates_same_file() -> TestResult {
    let known = paths(&[
        "src/main.rs",
        "src/a.rs",
        "src/b.rs",
        "src/model/mod.rs",
        "src/foo/mod.rs",
        "src/foo/bar.rs",
        "src/foo/baz.rs",
    ]);
    let edges = resolve_imports(
        "rust",
        "src/main.rs",
        &[
            "use crate::{a, b};".to_owned(),
            "use crate::model::{Span, LineIndex};".to_owned(),
            "use crate::foo::{self, bar, baz::qux as q, *};".to_owned(),
        ],
        &known,
    );

    let targets: Vec<Option<&str>> = edges
        .iter()
        .map(|edge| edge.to_relative_path.as_deref())
        .collect();
    ensure_eq(
        &targets,
        &vec![
            Some("src/a.rs"),
            Some("src/b.rs"),
            Some("src/model/mod.rs"),
            Some("src/foo/mod.rs"),
            Some("src/foo/bar.rs"),
            Some("src/foo/baz.rs"),
        ],
        "grouped use emits distinct files and deduplicates Span+LineIndex and self+* to mod.rs",
    )?;
    Ok(())
}

#[test]
fn grouped_rust_use_deduplicates_unresolved_leaves_but_keeps_resolved_ones() -> TestResult {
    let known = paths(&["src/caller.rs", "src/caller/existing.rs"]);
    let missing_group = resolve_imports(
        "rust",
        "src/caller.rs",
        &["use crate::missing::{A, B};".to_owned()],
        &known,
    );
    ensure_eq(
        &missing_group.len(),
        &1,
        "identical unresolved leaves collapse to one edge",
    )?;
    let only_missing = missing_group
        .first()
        .check("first unresolved edge for missing group exists")?;
    ensure_eq(
        &only_missing.resolution_kind,
        &"Unresolved",
        "collapsed missing group resolves as Unresolved",
    )?;
    ensure_eq(
        &only_missing.syntax_unmodeled,
        &false,
        "modeled missing group is not flagged as unmodeled syntax",
    )?;

    let mixed = resolve_imports(
        "rust",
        "src/caller.rs",
        &["use crate::{caller::existing, missing::{A, B}};".to_owned()],
        &known,
    );
    ensure_eq(
        &mixed.len(),
        &2,
        "mixed group emits one resolved edge plus one unresolved edge",
    )?;
    ensure_eq(
        &mixed
            .first()
            .check("first edge in mixed group exists")?
            .to_relative_path
            .as_deref(),
        &Some("src/caller/existing.rs"),
        "first edge in mixed group resolves to src/caller/existing.rs",
    )?;
    ensure_eq(
        &mixed
            .get(1)
            .check("second edge in mixed group exists")?
            .resolution_kind,
        &"Unresolved",
        "second edge in mixed group is Unresolved",
    )?;
    Ok(())
}

#[test]
fn empty_brace_use_emits_no_edges_while_unmodeled_syntax_is_flagged() -> TestResult {
    let known = paths(&["src/main.rs", "src/a.rs", "src/b.rs"]);
    let empty = resolve_imports(
        "rust",
        "src/main.rs",
        &["use crate::foo::{};".to_owned()],
        &known,
    );
    ensure_eq(&empty.len(), &0, "empty brace group emits zero edges")?;

    let unmodeled = resolve_imports("rust", "src/main.rs", &["pubfn use x;".to_owned()], &known);
    ensure_eq(
        &unmodeled.len(),
        &1,
        "malformed use emits one Unresolved edge",
    )?;
    ensure_eq(
        &unmodeled
            .first()
            .check("first edge for malformed use exists")?
            .syntax_unmodeled,
        &true,
        "malformed use is flagged as unmodeled syntax",
    )?;

    let root_group = resolve_imports(
        "rust",
        "src/sub/mod.rs",
        &["use {crate::a, super::b};".to_owned()],
        &known,
    );
    let root_targets: Vec<Option<&str>> = root_group
        .iter()
        .map(|edge| edge.to_relative_path.as_deref())
        .collect();
    ensure_eq(
        &root_targets,
        &vec![Some("src/a.rs"), Some("src/b.rs")],
        "root brace group resolves both crate and super branches",
    )?;
    Ok(())
}

#[test]
fn same_file_dedup_prefers_higher_confidence() -> TestResult {
    let known = paths(&["src/lib.rs", "src/other.rs"]);
    let edges = resolve_imports(
        "rust",
        "src/other.rs",
        &["use crate::{Item, self};".to_owned()],
        &known,
    );
    ensure_eq(
        &edges.len(),
        &1,
        "duplicate targets for the crate root collapse to one edge",
    )?;
    let only = edges.first().check("deduplicated edge exists")?;
    ensure_eq(
        &only.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "collapsed edge points to src/lib.rs",
    )?;
    ensure_eq(
        &only.confidence,
        &Some("High"),
        "deduplication prefers High confidence over Low confidence",
    )?;
    Ok(())
}

#[test]
fn resolve_rust_mod_declaration_handles_files_mod_rs_inline_path_attr_and_raw_idents() -> TestResult
{
    let known = paths(&[
        "src/lib.rs",
        "src/foo.rs",
        "src/bar/mod.rs",
        "src/foo/a/b.rs",
        "src/foo_tests.rs",
        "src/foo/inner/x.rs",
        "src/type.rs",
        "benches/discovery.rs",
        "benches/common/mod.rs",
    ]);

    let file_edge = resolve_rust_mod_declaration("src/lib.rs", "foo", None, &[], &known);
    ensure_eq(
        &file_edge,
        &ResolvedEdge {
            raw_import_text: "mod foo;".to_owned(),
            resolution_kind: "Resolved",
            confidence: Some("High"),
            to_relative_path: Some("src/foo.rs".to_owned()),
            syntax_unmodeled: false,
        },
        "mod foo; in src/lib.rs resolves to src/foo.rs",
    )?;

    let dir_edge = resolve_rust_mod_declaration("src/lib.rs", "bar", None, &[], &known);
    ensure_eq(
        &dir_edge.to_relative_path.as_deref(),
        &Some("src/bar/mod.rs"),
        "mod bar; in src/lib.rs resolves to src/bar/mod.rs",
    )?;

    let nested_inline =
        resolve_rust_mod_declaration("src/foo.rs", "b", None, &["a".to_owned()], &known);
    ensure_eq(
        &nested_inline.to_relative_path.as_deref(),
        &Some("src/foo/a/b.rs"),
        "nested inline mod a { mod b; } in src/foo.rs resolves to src/foo/a/b.rs",
    )?;

    let sibling_path_attr =
        resolve_rust_mod_declaration("src/foo.rs", "tests", Some("foo_tests.rs"), &[], &known);
    ensure_eq(
        &sibling_path_attr.to_relative_path.as_deref(),
        &Some("src/foo_tests.rs"),
        "#[path = \"foo_tests.rs\"] mod tests; in src/foo.rs resolves relative to src/",
    )?;

    let inline_path_attr = resolve_rust_mod_declaration(
        "src/foo.rs",
        "y",
        Some("x.rs"),
        &["inner".to_owned()],
        &known,
    );
    ensure_eq(
        &inline_path_attr.to_relative_path.as_deref(),
        &Some("src/foo/inner/x.rs"),
        "inline mod inner { #[path = \"x.rs\"] mod y; } in src/foo.rs resolves under src/foo/inner/",
    )?;

    let raw_ident = resolve_rust_mod_declaration("src/lib.rs", "r#type", None, &[], &known);
    ensure_eq(
        &raw_ident,
        &ResolvedEdge {
            raw_import_text: "mod r#type;".to_owned(),
            resolution_kind: "Resolved",
            confidence: Some("High"),
            to_relative_path: Some("src/type.rs".to_owned()),
            syntax_unmodeled: false,
        },
        "mod r#type; strips r# for file lookup while preserving raw_import_text",
    )?;

    let shared_bench =
        resolve_rust_mod_declaration("benches/discovery.rs", "common", None, &[], &known);
    ensure_eq(
        &shared_bench.to_relative_path.as_deref(),
        &Some("benches/common/mod.rs"),
        "mod common; in benches/discovery.rs resolves shared benches/common/mod.rs",
    )?;

    let missing = resolve_rust_mod_declaration("src/lib.rs", "nonexistent", None, &[], &known);
    ensure_eq(
        &missing,
        &ResolvedEdge {
            raw_import_text: "mod nonexistent;".to_owned(),
            resolution_kind: "Unresolved",
            confidence: None,
            to_relative_path: None,
            syntax_unmodeled: false,
        },
        "missing module target resolves as Unresolved without unmodeled flag",
    )?;

    let dual_known = paths(&["src/lib.rs", "src/foo.rs", "src/foo/mod.rs"]);
    let dual_edge = resolve_rust_mod_declaration("src/lib.rs", "foo", None, &[], &dual_known);
    ensure_eq(
        &(
            dual_edge.resolution_kind,
            dual_edge.to_relative_path.as_deref(),
            dual_edge.confidence,
        ),
        &("Resolved", Some("src/foo.rs"), Some("Low")),
        "mod foo; with both src/foo.rs and src/foo/mod.rs present stamps Low confidence",
    )?;
    Ok(())
}
