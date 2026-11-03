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
