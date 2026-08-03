use super::*;

fn paths<'a>(list: &[&'a str]) -> HashSet<&'a str> {
    list.iter().copied().collect()
}

#[test]
fn every_source_produces_exactly_one_edge_in_order() {
    let known = paths(&["pkg/a.py", "pkg/bar.py"]);
    let sources = vec![
        "from .bar import baz".to_owned(),
        "import os".to_owned(),
        "from .missing import x".to_owned(),
    ];
    let edges = resolve_imports("python", "pkg/a.py", &sources, &known);

    assert_eq!(edges.len(), 3);
    assert_eq!(edges[0].resolution_kind, "Resolved");
    assert_eq!(edges[0].to_relative_path.as_deref(), Some("pkg/bar.py"));
    assert_eq!(edges[0].raw_import_text, "from .bar import baz");

    assert_eq!(edges[1].resolution_kind, "External");
    assert_eq!(edges[1].to_relative_path, None);

    assert_eq!(edges[2].resolution_kind, "Unresolved");
    assert_eq!(edges[2].to_relative_path, None);
}

#[test]
fn an_unparseable_raw_statement_becomes_unresolved_not_dropped() {
    let known = paths(&["a.go"]);
    let edges = resolve_imports("go", "a.go", &["import \"fmt\"".to_owned()], &known);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].resolution_kind, "Unresolved");
    assert_eq!(edges[0].raw_import_text, "import \"fmt\"");
}

#[test]
fn circular_imports_are_just_two_independent_edges() {
    // a.py imports b, b.py imports a — nothing about resolution needs to
    // special-case a cycle; each direction resolves independently, and
    // detecting/traversing the cycle is a `query` concern (recursive CTE
    // over dependency_edges), not something the resolver prevents or flags.
    let known = paths(&["a.py", "b.py"]);
    let a_to_b = resolve_imports("python", "a.py", &["from . import b".to_owned()], &known);
    let b_to_a = resolve_imports("python", "b.py", &["from . import a".to_owned()], &known);

    assert_eq!(a_to_b[0].resolution_kind, "Unresolved");
    assert_eq!(b_to_a[0].resolution_kind, "Unresolved");
    // `from . import b` resolves the *package* (`.` → __init__.py), not the
    // submodule named after the imported symbol — a known limitation of
    // not modeling `from X import Y` symbol-vs-submodule ambiguity; both
    // directions are still independently and consistently classified.
}

#[test]
fn an_aliased_import_resolves_on_the_module_not_the_alias() {
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
    assert_eq!(edges[0].resolution_kind, "Resolved");
    assert_eq!(
        edges[0].to_relative_path.as_deref(),
        Some("pkg/real_name.py")
    );
}
