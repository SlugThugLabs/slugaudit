use super::edge_test_harness::{EdgeTestHarness, StoredEdge};
use super::revision_test_fixtures::{
    import_evidence, lib_record, mod_evidence, rust_record, unparsed_record,
};
use crate::graph::{RustModuleParents, WorkspaceCrates};
use crate::sync::FileRecord;
use crate::sync::revision_edge_match::low_conf_edge_affected;
use crate::sync::rust_super_paths::{ChangedRustPaths, RustScopeView};
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq, ensure_ne};
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq)]
struct FullEdgeRow {
    from: String,
    to: Option<String>,
    raw_import_text: String,
    kind: String,
    confidence: Option<String>,
    syntax_unmodeled: bool,
}

fn pkg_toml(path: &str, hash: &str, name: &str, extra: &str) -> FileRecord {
    let body = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n{extra}");
    unparsed_record(path, hash, &body, None)
}

fn load_full_edges(conn: &Connection) -> TestResult<Vec<FullEdgeRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT f.path, t.path, e.raw_import_text, e.resolution_kind, e.confidence, \
             e.syntax_unmodeled FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             LEFT JOIN files t ON t.id = e.to_file_id \
             ORDER BY f.path, COALESCE(t.path, ''), e.raw_import_text, e.resolution_kind, \
             COALESCE(e.confidence, '')",
        )
        .check("prepare full edge query")?;
    stmt.query_map([], |row| {
        Ok(FullEdgeRow {
            from: row.get(0)?,
            to: row.get(1)?,
            raw_import_text: row.get(2)?,
            kind: row.get(3)?,
            confidence: row.get(4)?,
            syntax_unmodeled: row.get(5)?,
        })
    })
    .check("run full edge query")?
    .collect::<Result<_, _>>()
    .check("collect full edges")
}

fn assert_cold_parity(
    h: &EdgeTestHarness,
    final_records: &[FileRecord],
    context: &str,
) -> TestResult {
    let mut cold = EdgeTestHarness::new()?;
    cold.publish(final_records, false)?;
    ensure_eq(
        &load_full_edges(h.connection())?,
        &load_full_edges(cold.connection())?,
        context,
    )
}

#[test]
fn corner1_external_import_reresolves_when_src_tests_or_bin_submodule_added() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let root_toml = pkg_toml("Cargo.toml", "ht1", "demo", "");
    let lib = rust_record(
        "src/lib.rs",
        "hl1",
        vec![
            import_evidence(0, "use tests::unit;"),
            import_evidence(1, "use bin::helper;"),
        ],
    );
    let pkg_toml_rec = pkg_toml("crates/pkg/Cargo.toml", "hpt1", "custom-name", "");
    let pkg_lib = rust_record(
        "crates/pkg/src/lib.rs",
        "hpl1",
        vec![import_evidence(0, "use pkg::unit;")],
    );
    h.publish(
        &[
            root_toml.clone(),
            lib.clone(),
            pkg_toml_rec.clone(),
            pkg_lib.clone(),
        ],
        false,
    )?;
    h.assert_edges(
        &[
            StoredEdge::external("crates/pkg/src/lib.rs"),
            StoredEdge::external("src/lib.rs"),
            StoredEdge::external("src/lib.rs"),
        ],
        "before submodules exist, tests::unit, bin::helper, and pkg::unit are External",
    )?;
    let pkg_ids_before = h.edge_row_ids_for("crates/pkg/src/lib.rs")?;

    let src_tests_unit = rust_record("src/tests/unit.rs", "hsu1", vec![]);
    let src_bin_helper = rust_record("src/bin/helper.rs", "hsbh1", vec![]);
    let pkg_tests_unit = rust_record("crates/pkg/tests/unit.rs", "hptu1", vec![]);
    h.publish(
        &[
            src_tests_unit.clone(),
            src_bin_helper.clone(),
            pkg_tests_unit.clone(),
        ],
        true,
    )?;

    h.assert_edge_row_ids_unchanged(
        "crates/pkg/src/lib.rs",
        &pkg_ids_before,
        "Cargo.toml boundary stops stem walk before inserting pkg directory name",
    )?;
    h.assert_edges(
        &[
            StoredEdge::external("crates/pkg/src/lib.rs"),
            StoredEdge::resolved("src/lib.rs", "src/bin/helper.rs"),
            StoredEdge::resolved("src/lib.rs", "src/tests/unit.rs"),
        ],
        "adding src/tests/unit.rs and src/bin/helper.rs upgrades unchanged External imports",
    )?;
    let final_set = [
        root_toml,
        lib,
        pkg_toml_rec,
        pkg_lib,
        src_tests_unit,
        src_bin_helper,
        pkg_tests_unit,
    ];
    assert_cold_parity(&h, &final_set, "corner1 incremental matches cold publish")
}

#[test]
fn corner2_super_resolves_through_path_attr_and_custom_crate_root_on_delete() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let toml = pkg_toml(
        "Cargo.toml",
        "ht1",
        "demo",
        "\n[[bin]]\nname = \"cli\"\npath = \"bins/cli_entry.rs\"\n",
    );
    let lib = lib_record();
    let host = rust_record(
        "src/a/mod.rs",
        "hh1",
        vec![mod_evidence(0, "bar", Some("../shared/bar.rs"))],
    );
    let bar = rust_record(
        "src/shared/bar.rs",
        "hb1",
        vec![mod_evidence(0, "baz", None)],
    );
    let baz = rust_record(
        "src/shared/bar/baz.rs",
        "hbz1",
        vec![import_evidence(0, "use super::super::sibling;")],
    );
    let sibling = rust_record("src/a/sibling.rs", "hs1", vec![]);
    let cli_entry = rust_record("bins/cli_entry.rs", "hce1", vec![]);
    let bin_sub = rust_record(
        "bins/sub.rs",
        "hbs1",
        vec![import_evidence(0, "use super::RootFlag;")],
    );

    let initial = [
        toml.clone(),
        lib.clone(),
        host.clone(),
        bar.clone(),
        baz.clone(),
        sibling,
        cli_entry,
        bin_sub.clone(),
    ];
    h.publish(&initial, false)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("bins/sub.rs", "bins/cli_entry.rs"),
            StoredEdge::resolved("src/a/mod.rs", "src/shared/bar.rs"),
            StoredEdge::resolved("src/shared/bar.rs", "src/shared/bar/baz.rs"),
            StoredEdge::resolved("src/shared/bar/baz.rs", "src/a/sibling.rs"),
        ],
        "initial publish resolves #[path] super::super and custom [[bin]] super:: edges",
    )?;

    h.publish_with_deletions(
        &[],
        &[
            "src/a/sibling.rs".to_owned(),
            "bins/cli_entry.rs".to_owned(),
        ],
        true,
    )?;
    assert_cold_parity(
        &h,
        &[toml, lib, host, bar, baz, bin_sub],
        "deleting #[path] super target and custom [[bin]] root matches cold publish",
    )
}

#[test]
fn corner3_pure_super_low_conf_edge_affected_on_target_dir_change() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let lib = lib_record();
    let a_mod = rust_record("src/a/mod.rs", "ham1", vec![]);
    let a_sub = rust_record(
        "src/a/sub.rs",
        "has1",
        vec![import_evidence(0, "use super::missing::Item;")],
    );
    let b_mod = rust_record("src/b/mod.rs", "hbm1", vec![]);
    let b_other = rust_record(
        "src/b/other.rs",
        "hbo1",
        vec![import_evidence(0, "use super::missing::Item;")],
    );

    h.publish(
        &[
            lib.clone(),
            a_mod.clone(),
            a_sub.clone(),
            b_mod.clone(),
            b_other.clone(),
        ],
        false,
    )?;
    let a_ids_before = h.edge_row_ids_for("src/a/sub.rs")?;
    let b_ids_before = h.edge_row_ids_for("src/b/other.rs")?;

    let a_extra = rust_record("src/a/extra.rs", "hae1", vec![]);
    h.publish(std::slice::from_ref(&a_extra), true)?;
    ensure_ne(
        &h.edge_row_ids_for("src/a/sub.rs")?,
        &a_ids_before,
        "pure super:: Low edge in src/a is invalidated when src/a/extra.rs is added",
    )?;
    h.assert_edge_row_ids_unchanged(
        "src/b/other.rs",
        &b_ids_before,
        "unrelated src/b Low edge untouched",
    )?;
    assert_cold_parity(
        &h,
        &[lib, a_mod, a_sub, b_mod, b_other, a_extra],
        "corner3 parity",
    )?;

    let known: HashSet<&str> = ["src/a/mod.rs", "src/a/sub.rs"].into_iter().collect();
    let (parents, crates) = (RustModuleParents::new(), WorkspaceCrates::new());
    let view = RustScopeView {
        known_paths: &known,
        rust_parents: &parents,
        workspace_crates: &crates,
    };
    let same_paths = ["src/a/extra.rs".to_owned()];
    let other_paths = ["src/b/extra.rs".to_owned()];
    let same_dir = ChangedRustPaths::new(&same_paths, &[]);
    let other_dir = ChangedRustPaths::new(&other_paths, &[]);
    ensure(
        low_conf_edge_affected(
            "src/a/sub.rs",
            Some("src/a/mod.rs"),
            "use super::missing::Item;",
            &same_dir,
            &view,
        ),
        "pure super:: Low edge returns true when target dir changes",
    )?;
    ensure(
        !low_conf_edge_affected(
            "src/a/sub.rs",
            Some("src/a/mod.rs"),
            "use super::missing::Item;",
            &other_dir,
            &view,
        ),
        "pure super:: Low edge returns false for unrelated dir change",
    )
}

#[test]
fn corner4_ordinary_submodule_of_path_attr_child_updates_across_host_edits() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let crate_toml = pkg_toml("crates/a/Cargo.toml", "hat1", "crate-a", "");
    let host_plain = rust_record("crates/a/src/lib.rs", "hl1", vec![]);
    let thing = rust_record("crates/a/src/thing.rs", "htg1", vec![]);
    let shared_sub = rust_record("shared/sub.rs", "hss1", vec![mod_evidence(0, "leaf", None)]);
    let shared_leaf = rust_record(
        "shared/sub/leaf.rs",
        "hsl1",
        vec![import_evidence(0, "use crate::thing::Thing;")],
    );

    h.publish(
        &[
            crate_toml.clone(),
            host_plain,
            thing.clone(),
            shared_sub.clone(),
            shared_leaf.clone(),
        ],
        false,
    )?;
    h.assert_edges(
        &[
            StoredEdge::resolved("shared/sub.rs", "shared/sub/leaf.rs"),
            StoredEdge::unresolved("shared/sub/leaf.rs"),
        ],
        "before host adds #[path], ordinary child shared/sub/leaf.rs is Unresolved",
    )?;

    let host_with_path = rust_record(
        "crates/a/src/lib.rs",
        "hl2",
        vec![mod_evidence(0, "sub", Some("../../../shared/sub.rs"))],
    );
    h.publish(std::slice::from_ref(&host_with_path), false)?;
    assert_cold_parity(
        &h,
        &[
            crate_toml.clone(),
            host_with_path,
            thing.clone(),
            shared_sub.clone(),
            shared_leaf.clone(),
        ],
        "adding #[path] to host updates ordinary submodule shared/sub/leaf.rs matching cold publish",
    )?;

    let leaf_ids_before = h.edge_row_ids_for("shared/sub/leaf.rs")?;
    let extra = rust_record("crates/a/src/extra.rs", "hae1", vec![]);
    h.publish(std::slice::from_ref(&extra), true)?;
    ensure_ne(
        &h.edge_row_ids_for("shared/sub/leaf.rs")?,
        &leaf_ids_before,
        "manifest/rust add re-resolves ordinary submodule reaching #[path] host",
    )?;

    let host_dropped = rust_record("crates/a/src/lib.rs", "hl3", vec![]);
    h.publish(std::slice::from_ref(&host_dropped), false)?;
    assert_cold_parity(
        &h,
        &[
            crate_toml,
            host_dropped,
            thing,
            shared_sub,
            shared_leaf,
            extra,
        ],
        "dropping #[path] from host reverts ordinary submodule shared/sub/leaf.rs matching cold publish",
    )
}

#[test]
fn corner5_named_target_main_rs_updates_path_attr_anchored_child() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let toml = pkg_toml("Cargo.toml", "ht1", "demo", "");
    let shared_common = rust_record("tests/common/mod.rs", "hcm1", vec![]);
    let it_helpers = rust_record(
        "tests/it/helpers.rs",
        "hih1",
        vec![mod_evidence(
            0,
            "helper_mod",
            Some("../../shared/helper_mod.rs"),
        )],
    );
    let helper_mod = rust_record(
        "shared/helper_mod.rs",
        "hhm1",
        vec![
            mod_evidence(0, "sub", None),
            import_evidence(0, "use crate::common::Setup;"),
        ],
    );
    let helper_sub = rust_record(
        "shared/helper_mod/sub.rs",
        "hhs1",
        vec![import_evidence(0, "use crate::common::Setup;")],
    );

    h.publish(
        &[
            toml.clone(),
            shared_common.clone(),
            it_helpers.clone(),
            helper_mod.clone(),
            helper_sub.clone(),
        ],
        false,
    )?;
    h.assert_edges(
        &[
            StoredEdge::resolved("shared/helper_mod.rs", "shared/helper_mod/sub.rs"),
            StoredEdge::resolved("shared/helper_mod.rs", "tests/common/mod.rs"),
            StoredEdge::resolved("shared/helper_mod/sub.rs", "tests/common/mod.rs"),
            StoredEdge::resolved("tests/it/helpers.rs", "shared/helper_mod.rs"),
        ],
        "without tests/it/main.rs, external #[path] child and its submodule resolve to tests/common/mod.rs",
    )?;

    let it_main = rust_record("tests/it/main.rs", "him1", vec![]);
    let it_common = rust_record("tests/it/common.rs", "hic1", vec![]);
    h.publish(&[it_main.clone(), it_common.clone()], true)?;
    assert_cold_parity(
        &h,
        &[
            toml.clone(),
            shared_common.clone(),
            it_helpers.clone(),
            helper_mod.clone(),
            helper_sub.clone(),
            it_main,
            it_common.clone(),
        ],
        "adding tests/it/main.rs re-anchors external #[path] child and submodule matching cold publish",
    )?;

    h.publish_with_deletions(&[], &["tests/it/main.rs".to_owned()], true)?;
    assert_cold_parity(
        &h,
        &[
            toml,
            shared_common,
            it_helpers,
            helper_mod,
            helper_sub,
            it_common,
        ],
        "deleting tests/it/main.rs reverts external #[path] child and submodule matching cold publish",
    )
}
