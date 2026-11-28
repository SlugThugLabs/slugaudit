pub(super) use super::edge_test_harness::{EdgeTestHarness, StoredEdge};
pub(super) use super::revision_test_fixtures::{
    file_record, import_evidence, import_evidence_in_inline_mod, lib_record, mod_evidence,
    rust_record, rust_run,
};

use super::revision_test_fixtures::unparsed_record;
use crate::sync::FileRecord;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

fn foo_tests_super() -> FileRecord {
    rust_record(
        "src/foo_tests.rs",
        "ht1",
        vec![import_evidence(0, "use super::*;")],
    )
}

fn foo_with_path(hash: &str) -> FileRecord {
    rust_record(
        "src/foo.rs",
        hash,
        vec![mod_evidence(0, "tests", Some("foo_tests.rs"))],
    )
}

fn foo_path_harness() -> TestResult<EdgeTestHarness> {
    let mut h = EdgeTestHarness::new()?;
    h.publish(
        &[foo_tests_super(), lib_record(), foo_with_path("hf1")],
        false,
    )?;
    Ok(h)
}

#[test]
fn grouped_use_resolves_incrementally_across_revisions_without_pop_loss() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let caller = rust_record(
        "src/caller.rs",
        "h1",
        vec![
            import_evidence(0, "use crate::{a, b};"),
            import_evidence(1, "use crate::foo::{};"),
            import_evidence(2, "pubfn use broken;"),
        ],
    );
    h.publish(&[caller, rust_record("src/a.rs", "h2", vec![])], false)?;
    h.assert_edges(
        &[
            StoredEdge::unresolved("src/caller.rs"),
            StoredEdge::unmodeled("src/caller.rs"),
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
        ],
        "initial publish emits resolved a.rs, unresolved b, and unmodeled broken edge",
    )?;

    h.publish(&[rust_record("src/b.rs", "h3", vec![])], true)?;
    h.assert_edges(
        &[
            StoredEdge::unmodeled("src/caller.rs"),
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
            StoredEdge::resolved("src/caller.rs", "src/b.rs"),
        ],
        "incremental publish re-resolves unchanged caller.rs to both a.rs and b.rs",
    )
}

#[test]
fn adding_rust_module_upgrades_low_confidence_crate_root_edge() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let caller = rust_record(
        "src/caller.rs",
        "hc1",
        vec![import_evidence(0, "use crate::{a, b};")],
    );
    h.publish(
        &[lib_record(), caller, rust_record("src/a.rs", "ha1", vec![])],
        false,
    )?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
            StoredEdge::resolved("src/caller.rs", "src/lib.rs"),
        ],
        "while b.rs is absent, crate::b falls back to the crate root with Low confidence",
    )?;

    h.publish(&[rust_record("src/b.rs", "hb1", vec![])], true)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
            StoredEdge::resolved("src/caller.rs", "src/b.rs"),
        ],
        "adding b.rs upgrades unchanged caller.rs edge from lib.rs to b.rs",
    )
}

#[test]
fn path_attr_super_edges_update_when_host_added_or_modified() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    h.publish(&[foo_tests_super(), lib_record()], false)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/foo_tests.rs", "src/lib.rs")],
        "without a declaring file, super::* falls back to the crate root",
    )?;

    h.publish(&[rust_record("src/foo.rs", "hf1", vec![])], true)?;
    h.publish(&[foo_with_path("hf2")], false)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/foo.rs", "src/foo_tests.rs"),
            StoredEdge::resolved("src/foo_tests.rs", "src/foo.rs"),
        ],
        "modifying foo.rs to declare #[path] emits mod edge and re-resolves foo_tests.rs to foo.rs",
    )
}

#[test]
fn editing_rust_file_with_unchanged_path_attr_does_not_reresolve_dependents() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let other_super = rust_record(
        "src/other.rs",
        "ho1",
        vec![import_evidence(0, "use super::*;")],
    );
    h.publish(
        &[
            lib_record(),
            foo_with_path("hf1"),
            foo_tests_super(),
            other_super,
        ],
        false,
    )?;
    let foo_tests_ids_before = h.edge_row_ids_for("src/foo_tests.rs")?;
    let other_ids_before = h.edge_row_ids_for("src/other.rs")?;

    // Edit foo.rs body (new hash) while keeping the exact same #[path = "foo_tests.rs"] mod tests;
    h.publish(&[foo_with_path("hf2")], false)?;

    h.assert_edge_row_ids_unchanged(
        "src/foo_tests.rs",
        &foo_tests_ids_before,
        "unchanged #[path] attribute on foo.rs does not rewrite child foo_tests.rs edges",
    )?;
    h.assert_edge_row_ids_unchanged(
        "src/other.rs",
        &other_ids_before,
        "unchanged #[path] attribute on foo.rs does not trigger full-repo super::* re-resolution",
    )
}

#[test]
fn adding_or_deleting_non_rust_file_does_not_reresolve_rust_dependents() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let caller = rust_record(
        "src/caller.rs",
        "hc1",
        vec![
            import_evidence(0, "use crate::missing_mod::Item;"),
            import_evidence(1, "use super::*;"),
        ],
    );
    h.publish(&[lib_record(), caller], false)?;
    let caller_ids_before = h.edge_row_ids_for("src/caller.rs")?;

    let notes = unparsed_record("NOTES.md", "hn1", "# notes\n", Some("prose"));
    h.publish(&[notes], true)?;
    h.assert_edge_row_ids_unchanged(
        "src/caller.rs",
        &caller_ids_before,
        "adding NOTES.md does not re-resolve Rust crates or low-confidence edges",
    )?;

    h.publish_with_deletions(&[], &["NOTES.md".to_owned()], true)?;
    h.assert_edge_row_ids_unchanged(
        "src/caller.rs",
        &caller_ids_before,
        "deleting NOTES.md does not re-resolve Rust crates or low-confidence edges",
    )
}

#[test]
fn path_attr_super_edges_revert_when_host_drops_attr() -> TestResult {
    let mut h = foo_path_harness()?;
    h.publish(&[rust_record("src/foo.rs", "hf2", vec![])], false)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/foo_tests.rs", "src/lib.rs")],
        "removing #[path] from foo.rs clears mod edge and reverts foo_tests.rs to lib.rs",
    )
}

#[test]
fn path_attr_super_edges_become_unresolved_when_host_and_root_deleted() -> TestResult {
    let mut h = foo_path_harness()?;
    h.publish_with_deletions(
        &[],
        &["src/foo.rs".to_owned(), "src/lib.rs".to_owned()],
        true,
    )?;
    h.assert_edges(
        &[StoredEdge::unresolved("src/foo_tests.rs")],
        "deleting the declaring file and crate root reverts child edge to Unresolved",
    )
}

#[test]
fn removing_path_attr_updates_child_super_super_edge_pointing_elsewhere() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let parent_with_path = rust_record(
        "src/a/parent.rs",
        "hp1",
        vec![mod_evidence(0, "tests", Some("../child.rs"))],
    );
    let child = rust_record(
        "src/child.rs",
        "hc1",
        vec![import_evidence(0, "use super::super::target::Item;")],
    );
    let target_under_a = rust_record("src/a/target.rs", "ht1", vec![]);
    h.publish(&[parent_with_path, child, target_under_a], false)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/a/parent.rs", "src/child.rs"),
            StoredEdge::resolved("src/child.rs", "src/a/target.rs"),
        ],
        "while parent.rs declares child.rs via #[path], parent->child and child->target both resolve",
    )?;

    h.publish(&[rust_record("src/a/parent.rs", "hp2", vec![])], false)?;
    h.assert_edges(
        &[StoredEdge::unresolved("src/child.rs")],
        "when parent.rs drops #[path] while child.rs is unchanged, stale super::super edge is cleared",
    )
}

#[test]
fn external_mod_declarations_emit_edges_and_reresolve_when_target_added() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    h.publish(
        &[rust_record(
            "src/lib.rs",
            "hl1",
            vec![mod_evidence(0, "sub", None)],
        )],
        false,
    )?;
    h.assert_edges(
        &[StoredEdge::unresolved("src/lib.rs")],
        "external mod sub; is Unresolved before src/sub.rs exists",
    )?;

    h.publish(&[rust_record("src/sub.rs", "hs1", vec![])], true)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/lib.rs", "src/sub.rs")],
        "adding src/sub.rs upgrades unchanged lib.rs mod sub; edge to Resolved",
    )
}

#[test]
fn adding_nested_rust_module_without_parent_mod_updates_external_import() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let lib = rust_record(
        "src/lib.rs",
        "hl1",
        vec![import_evidence(0, "use foo::bar;")],
    );
    h.publish(&[lib], false)?;
    h.assert_edges(&[StoredEdge::external("src/lib.rs")], "initially External")?;
    h.publish(&[rust_record("src/foo/bar.rs", "hb1", vec![])], true)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/lib.rs", "src/foo/bar.rs")],
        "adding src/foo/bar.rs without src/foo.rs re-resolves use foo::bar; to Resolved",
    )
}

#[test]
fn deleting_workspace_crate_lib_root_downgrades_high_submodule_edge_to_external() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let a_toml = unparsed_record(
        "crates/a/Cargo.toml",
        "ht1",
        "[package]\nname = \"crate-a\"\n",
        None,
    );
    let a_lib = rust_record("crates/a/src/lib.rs", "hal1", vec![]);
    let a_foo = rust_record("crates/a/src/foo.rs", "haf1", vec![]);
    let b_lib = rust_record(
        "crates/b/src/lib.rs",
        "hbl1",
        vec![import_evidence(0, "use crate_a::foo;")],
    );
    h.publish(&[a_toml, a_lib, a_foo, b_lib], false)?;
    h.assert_edges(
        &[StoredEdge::resolved(
            "crates/b/src/lib.rs",
            "crates/a/src/foo.rs",
        )],
        "cross-crate High edge points to crates/a/src/foo.rs",
    )?;
    h.publish_with_deletions(&[], &["crates/a/src/lib.rs".to_owned()], true)?;
    h.assert_edges(
        &[StoredEdge::external("crates/b/src/lib.rs")],
        "deleting crates/a/src/lib.rs downgrades crates/b edge from foo.rs to External",
    )
}

#[test]
fn inline_mod_super_import_reresolves_when_submodule_added() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let foo = rust_record(
        "src/foo.rs",
        "hf1",
        vec![import_evidence_in_inline_mod(
            "import:0",
            "use super::sub::Item;",
            &["tests"],
        )],
    );
    h.publish(&[lib_record(), foo], false)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/foo.rs", "src/foo.rs")],
        "while src/foo/sub.rs is absent, inline mod super::sub::Item resolves to src/foo.rs",
    )?;

    h.publish(&[rust_record("src/foo/sub.rs", "hs1", vec![])], true)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/foo.rs", "src/foo/sub.rs")],
        "adding src/foo/sub.rs upgrades inline mod super::sub::Item to src/foo/sub.rs",
    )
}

#[test]
fn local_module_shadowing_workspace_crate_reresolves_from_high_to_low() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let a_toml = unparsed_record(
        "crates/a/Cargo.toml",
        "hat1",
        "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n",
        None,
    );
    let b_toml = unparsed_record(
        "crates/b/Cargo.toml",
        "hbt1",
        "[package]\nname = \"pkg-b\"\nversion = \"0.1.0\"\n\n[dependencies]\npkg-a = { path = \"../a\" }\n",
        None,
    );
    let a_lib = rust_record("crates/a/src/lib.rs", "hal1", vec![]);
    let a_bar = rust_record("crates/a/src/bar.rs", "hab1", vec![]);
    let b_lib = rust_record(
        "crates/b/src/lib.rs",
        "hbl1",
        vec![import_evidence(0, "use pkg_a::bar;")],
    );
    h.publish(&[a_toml, b_toml, a_lib, a_bar, b_lib], false)?;
    let edge_info = |harness: &EdgeTestHarness,
                     path: &str|
     -> TestResult<(Option<String>, String, Option<String>)> {
        harness
            .connection()
            .query_row(
                "SELECT t.path, e.resolution_kind, e.confidence \
                 FROM dependency_edges e \
                 JOIN files f ON f.id = e.from_file_id \
                 LEFT JOIN files t ON t.id = e.to_file_id \
                 WHERE f.path = ?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .check("query dependency edge")
    };
    ensure_eq(
        &edge_info(&h, "crates/b/src/lib.rs")?,
        &(
            Some("crates/a/src/bar.rs".to_owned()),
            "Resolved".to_owned(),
            Some("High".to_owned()),
        ),
        "initially resolves to crates/a/src/bar.rs with High confidence",
    )?;

    h.publish(
        &[rust_record("crates/b/src/pkg_a.rs", "hbpa1", vec![])],
        true,
    )?;
    ensure_eq(
        &edge_info(&h, "crates/b/src/lib.rs")?,
        &(
            Some("crates/b/src/pkg_a.rs".to_owned()),
            "Resolved".to_owned(),
            Some("Low".to_owned()),
        ),
        "adding crates/b/src/pkg_a.rs shadows pkg_a and re-resolves with Low confidence",
    )
}
