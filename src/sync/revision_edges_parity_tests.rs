use super::tests::{
    EdgeTestHarness, StoredEdge, file_record, import_evidence, lib_record, mod_evidence,
    rust_record, rust_run,
};
use crate::model::{
    ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun, SourceIdentity,
};
use crate::sync::FileRecord;
use crate::sync::revision::{SKIP_REASON_OVERSIZE, SkippedRecord};
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

fn toml_record(path: &str, hash: &str, content: &str) -> FileRecord {
    file_record(
        path,
        hash,
        content,
        None,
        ParserRun {
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::NotAttempted,
            completeness: ExtractionCompleteness::Unavailable,
        },
        Vec::new(),
    )
}

fn package_toml(path: &str, hash: &str, name: &str) -> FileRecord {
    toml_record(
        path,
        hash,
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
}

fn rust_record_from_source(path: &str, hash: &str, source: &str) -> TestResult<FileRecord> {
    let (evidence, truncated) = crate::evidence::extract("rust", source).check("extract rust")?;
    ensure(!truncated, "test source fixture should not truncate")?;
    Ok(file_record(
        path,
        hash,
        source,
        Some("rust"),
        rust_run(),
        evidence,
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct FullEdge {
    from: String,
    to: Option<String>,
    raw_import_text: String,
    kind: String,
    confidence: Option<String>,
    syntax_unmodeled: bool,
}

fn all_full_edges(conn: &Connection) -> TestResult<Vec<FullEdge>> {
    let mut stmt = conn
        .prepare(
            "SELECT f.path, t.path, e.raw_import_text, e.resolution_kind, e.confidence, e.syntax_unmodeled \
             FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             LEFT JOIN files t ON t.id = e.to_file_id \
             ORDER BY f.path, COALESCE(t.path, ''), e.raw_import_text, e.resolution_kind, COALESCE(e.confidence, '')",
        )
        .check("prepare full edge query")?;
    stmt.query_map([], |row| {
        Ok(FullEdge {
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

fn assert_cold_parity_with_skipped(
    h: &EdgeTestHarness,
    final_records: &[FileRecord],
    final_skipped: &[SkippedRecord],
    context: &str,
) -> TestResult {
    let mut cold = EdgeTestHarness::new()?;
    cold.publish_with_skipped(final_records, &[], final_skipped, false)?;
    let incremental_edges = all_full_edges(h.connection())?;
    let cold_edges = all_full_edges(cold.connection())?;
    ensure_eq(&incremental_edges, &cold_edges, context)
}

fn assert_cold_parity(
    h: &EdgeTestHarness,
    final_records: &[FileRecord],
    context: &str,
) -> TestResult {
    assert_cold_parity_with_skipped(h, final_records, &[], context)
}

#[test]
fn workspace_cargo_toml_resolves_cross_crate_imports_and_updates_on_edit() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let alpha_toml = package_toml("crates/alpha/Cargo.toml", "ht1", "my-alpha");
    let alpha_lib = rust_record("crates/alpha/src/lib.rs", "ha1", vec![]);
    let beta_lib = rust_record(
        "crates/beta/src/lib.rs",
        "hb1",
        vec![import_evidence(0, "use my_alpha::Widget;")],
    );
    h.publish(&[alpha_toml, alpha_lib, beta_lib], false)?;
    h.assert_edges(
        &[StoredEdge::resolved(
            "crates/beta/src/lib.rs",
            "crates/alpha/src/lib.rs",
        )],
        "hyphenated workspace package my-alpha resolves use my_alpha::Widget",
    )?;

    let renamed_toml = package_toml("crates/alpha/Cargo.toml", "ht2", "renamed-alpha");
    h.publish(&[renamed_toml], false)?;
    h.assert_edges(
        &[StoredEdge::external("crates/beta/src/lib.rs")],
        "editing Cargo.toml in place downgrades missing workspace crate to External",
    )?;

    let restored_toml = package_toml("crates/alpha/Cargo.toml", "ht3", "my-alpha");
    h.publish(&[restored_toml], false)?;
    h.assert_edges(
        &[StoredEdge::resolved(
            "crates/beta/src/lib.rs",
            "crates/alpha/src/lib.rs",
        )],
        "editing Cargo.toml back upgrades External cross-crate edge to Resolved",
    )
}

#[test]
fn multi_cfg_attr_path_targets_emit_all_edges_and_resolve_super_for_every_target() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let lib_multi = rust_record_from_source(
        "src/lib.rs",
        "hl1",
        concat!(
            "#[cfg_attr(unix, path = \"platform/unix.rs\")]\n",
            "#[cfg_attr(windows, path = \"platform/win.rs\")]\n",
            "mod os;\n",
        ),
    )?;
    let unix_rs = rust_record(
        "src/platform/unix.rs",
        "hu1",
        vec![import_evidence(0, "use super::*;")],
    );
    let win_rs = rust_record(
        "src/platform/win.rs",
        "hw1",
        vec![import_evidence(0, "use super::*;")],
    );

    h.publish(&[lib_multi, unix_rs.clone(), win_rs.clone()], false)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/lib.rs", "src/platform/unix.rs"),
            StoredEdge::resolved("src/lib.rs", "src/platform/win.rs"),
            StoredEdge::resolved("src/platform/unix.rs", "src/lib.rs"),
            StoredEdge::resolved("src/platform/win.rs", "src/lib.rs"),
        ],
        "both cfg_attr path targets emit mod edges and resolve super::* back to src/lib.rs",
    )?;

    let lib_unix_only = rust_record_from_source(
        "src/lib.rs",
        "hl2",
        "#[cfg_attr(unix, path = \"platform/unix.rs\")]\nmod os;\n",
    )?;
    h.publish(std::slice::from_ref(&lib_unix_only), false)?;
    assert_cold_parity(
        &h,
        &[lib_unix_only, unix_rs, win_rs],
        "dropping one cfg_attr path target matches cold publish",
    )
}

#[test]
fn cold_vs_incremental_parity_for_2a_external_relative_and_pending_workspace_imports() -> TestResult
{
    let mut h = EdgeTestHarness::new()?;
    let root_lib = rust_record(
        "src/lib.rs",
        "hl1",
        vec![import_evidence(0, "use sub::Helper;")],
    );
    let alpha_toml = package_toml("crates/alpha/Cargo.toml", "hat1", "my-alpha");
    let beta_lib = rust_record(
        "crates/beta/src/lib.rs",
        "hb1",
        vec![import_evidence(0, "use my_alpha::Widget;")],
    );
    let beta_other = rust_record(
        "crates/beta/src/other.rs",
        "hbo1",
        vec![import_evidence(0, "use serde::Serialize;")],
    );

    h.publish(
        &[
            root_lib.clone(),
            alpha_toml.clone(),
            beta_lib.clone(),
            beta_other.clone(),
        ],
        false,
    )?;
    h.assert_edges(
        &[
            StoredEdge::external("crates/beta/src/lib.rs"),
            StoredEdge::external("crates/beta/src/other.rs"),
            StoredEdge::external("src/lib.rs"),
        ],
        "before target files exist, 2018-edition relative and pending-workspace imports are External",
    )?;

    let root_sub = rust_record("src/sub.rs", "hs1", vec![]);
    let alpha_lib = rust_record("crates/alpha/src/lib.rs", "ha1", vec![]);
    h.publish(&[root_sub.clone(), alpha_lib.clone()], true)?;

    assert_cold_parity(
        &h,
        &[
            root_lib, alpha_toml, beta_lib, beta_other, root_sub, alpha_lib,
        ],
        "adding src/sub.rs and crates/alpha/src/lib.rs upgrades External imports to match cold publish",
    )
}

#[test]
fn cold_vs_incremental_parity_for_2b_cargo_toml_touched_updates_mod_only_rust_files() -> TestResult
{
    let mut h = EdgeTestHarness::new()?;
    let toml_v1 = package_toml("Cargo.toml", "ht1", "demo");
    let lib_rs = lib_record();
    let cli_rs = rust_record("src/cli.rs", "hc1", vec![mod_evidence(0, "helper", None)]);
    let sibling_helper = rust_record("src/helper.rs", "hh1", vec![]);
    let nested_helper = rust_record("src/cli/helper.rs", "hnh1", vec![]);

    h.publish(
        &[
            toml_v1,
            lib_rs.clone(),
            cli_rs.clone(),
            sibling_helper.clone(),
            nested_helper.clone(),
        ],
        false,
    )?;
    h.assert_edges(
        &[StoredEdge::resolved("src/cli.rs", "src/cli/helper.rs")],
        "before Cargo.toml declares [[bin]] src/cli.rs, mod helper; resolves to src/cli/helper.rs",
    )?;

    let toml_v2 = toml_record(
        "Cargo.toml",
        "ht2",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"cli\"\npath = \"src/cli.rs\"\n",
    );
    h.publish(std::slice::from_ref(&toml_v2), true)?;

    assert_cold_parity(
        &h,
        &[toml_v2, lib_rs, cli_rs, sibling_helper, nested_helper],
        "editing Cargo.toml to declare [[bin]] re-resolves mod-only src/cli.rs to match cold publish",
    )
}

#[test]
fn cold_vs_incremental_parity_for_2c_path_attr_host_deleted_skipped_or_edited_with_crate_child()
-> TestResult {
    let setup = || -> TestResult<(EdgeTestHarness, [FileRecord; 3])> {
        let crate_toml = package_toml("crates/a/Cargo.toml", "ht1", "crate-a");
        let host_with_path = rust_record(
            "crates/a/src/lib.rs",
            "hl1",
            vec![mod_evidence(0, "helper", Some("../../../shared/helper.rs"))],
        );
        let thing = rust_record("crates/a/src/thing.rs", "htg1", vec![]);
        let child_crate_only = rust_record(
            "shared/helper.rs",
            "hsh1",
            vec![import_evidence(0, "use crate::thing::Thing;")],
        );
        let mut h = EdgeTestHarness::new()?;
        h.publish(
            &[
                crate_toml.clone(),
                host_with_path,
                thing.clone(),
                child_crate_only.clone(),
            ],
            false,
        )?;
        Ok((h, [crate_toml, thing, child_crate_only]))
    };

    // Subcase A: host edited in place to drop #[path] while child only uses `crate::` (High confidence)
    let (mut h_edit, base_edit) = setup()?;
    h_edit.assert_edges(
        &[
            StoredEdge::resolved("crates/a/src/lib.rs", "shared/helper.rs"),
            StoredEdge::resolved("shared/helper.rs", "crates/a/src/thing.rs"),
        ],
        "initial publish links shared/helper.rs crate::thing to crates/a/src/thing.rs",
    )?;
    let host_without_path = rust_record("crates/a/src/lib.rs", "hl2", vec![]);
    h_edit.publish(std::slice::from_ref(&host_without_path), false)?;
    let [crate_toml, thing, child_crate_only] = base_edit;
    assert_cold_parity(
        &h_edit,
        &[crate_toml, host_without_path, thing, child_crate_only],
        "editing host to drop #[path] re-resolves High-confidence crate:: child to match cold publish",
    )?;

    // Subcase B: host deleted alongside an added file
    let (mut h_del, [crate_toml, thing, child_crate_only]) = setup()?;
    let extra = rust_record("crates/a/src/extra.rs", "he1", vec![]);
    h_del.publish_with_deletions(
        std::slice::from_ref(&extra),
        &["crates/a/src/lib.rs".to_owned()],
        true,
    )?;
    assert_cold_parity(
        &h_del,
        &[crate_toml, thing, child_crate_only, extra],
        "deleting #[path] host re-resolves High-confidence crate:: child to match cold publish",
    )?;

    // Subcase C: host transitions to skipped
    let (mut h_skip, base_skip) = setup()?;
    let skipped_host = SkippedRecord {
        relative_path: "crates/a/src/lib.rs".to_owned(),
        reason: SKIP_REASON_OVERSIZE.to_owned(),
        identity: SourceIdentity::new("crates/a/src/lib.rs".to_owned(), "stat-1".to_owned()),
        byte_len: Some(1_000_000),
        modified_unix_seconds: None,
    };
    h_skip.publish_with_skipped(&[], &[], std::slice::from_ref(&skipped_host), false)?;
    assert_cold_parity_with_skipped(
        &h_skip,
        &base_skip,
        &[skipped_host],
        "skipping #[path] host re-resolves High-confidence crate:: child to match cold publish",
    )
}

#[test]
fn cold_vs_incremental_parity_for_2d_ambiguous_sibling_and_tests_main_rs() -> TestResult {
    // Part 1: foo.rs <-> foo/mod.rs ambiguity
    let mut h_sib = EdgeTestHarness::new()?;
    let lib_mod_foo = rust_record("src/lib.rs", "hl1", vec![mod_evidence(0, "foo", None)]);
    let caller = rust_record(
        "src/caller.rs",
        "hc1",
        vec![import_evidence(0, "use crate::foo::Item;")],
    );
    let foo_mod = rust_record("src/foo/mod.rs", "hfm1", vec![]);
    h_sib.publish(
        &[lib_mod_foo.clone(), caller.clone(), foo_mod.clone()],
        false,
    )?;

    let foo_rs = rust_record("src/foo.rs", "hf1", vec![]);
    h_sib.publish(std::slice::from_ref(&foo_rs), true)?;
    assert_cold_parity(
        &h_sib,
        &[
            lib_mod_foo.clone(),
            caller.clone(),
            foo_mod.clone(),
            foo_rs.clone(),
        ],
        "adding src/foo.rs alongside src/foo/mod.rs matches cold publish (moves target + Low confidence)",
    )?;

    h_sib.publish_with_deletions(&[], &["src/foo.rs".to_owned()], true)?;
    assert_cold_parity(
        &h_sib,
        &[lib_mod_foo.clone(), caller.clone(), foo_mod],
        "deleting src/foo.rs restores High-confidence src/foo/mod.rs edges matching cold publish",
    )?;

    let mut h_rev = EdgeTestHarness::new()?;
    h_rev.publish(
        &[lib_mod_foo.clone(), caller.clone(), foo_rs.clone()],
        false,
    )?;
    let foo_mod_new = rust_record("src/foo/mod.rs", "hfm2", vec![]);
    h_rev.publish(std::slice::from_ref(&foo_mod_new), true)?;
    assert_cold_parity(
        &h_rev,
        &[lib_mod_foo, caller, foo_rs, foo_mod_new],
        "adding src/foo/mod.rs alongside src/foo.rs downgrades confidence to Low matching cold publish",
    )?;

    // Part 2: tests/<seg>/main.rs multi-file integration test target
    let mut h_tests = EdgeTestHarness::new()?;
    let root_toml = package_toml("Cargo.toml", "ht1", "demo");
    let shared_common = rust_record("tests/common/mod.rs", "hcm1", vec![]);
    let it_helpers = rust_record(
        "tests/it/helpers.rs",
        "hih1",
        vec![import_evidence(0, "use crate::common::Setup;")],
    );
    h_tests.publish(
        &[root_toml.clone(), shared_common.clone(), it_helpers.clone()],
        false,
    )?;
    h_tests.assert_edges(
        &[StoredEdge::resolved(
            "tests/it/helpers.rs",
            "tests/common/mod.rs",
        )],
        "without tests/it/main.rs, tests/it/helpers.rs resolves crate::common to tests/common/mod.rs",
    )?;

    let it_main = rust_record("tests/it/main.rs", "him1", vec![]);
    let it_common = rust_record("tests/it/common.rs", "hic1", vec![]);
    h_tests.publish(&[it_main.clone(), it_common.clone()], true)?;
    assert_cold_parity(
        &h_tests,
        &[
            root_toml.clone(),
            shared_common.clone(),
            it_helpers.clone(),
            it_main,
            it_common.clone(),
        ],
        "adding tests/it/main.rs re-anchors tests/it/helpers.rs to match cold publish",
    )?;

    h_tests.publish_with_deletions(&[], &["tests/it/main.rs".to_owned()], true)?;
    assert_cold_parity(
        &h_tests,
        &[root_toml, shared_common, it_helpers, it_common],
        "deleting tests/it/main.rs reverts tests/it/helpers.rs to tests/common/mod.rs matching cold publish",
    )
}

#[test]
fn scoping_super_dependents_avoids_unrelated_crates_and_directories() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let alpha_toml = package_toml("crates/alpha/Cargo.toml", "hat1", "alpha");
    let alpha_lib = rust_record("crates/alpha/src/lib.rs", "hal1", vec![]);
    let alpha_foo = rust_record(
        "crates/alpha/src/foo.rs",
        "haf1",
        vec![import_evidence(0, "use super::bar::Item;")],
    );
    let alpha_unrelated_tests = rust_record(
        "crates/alpha/src/unrelated/tests.rs",
        "haut1",
        vec![import_evidence(0, "use super::*;")],
    );
    let beta_toml = package_toml("crates/beta/Cargo.toml", "hbt1", "beta");
    let beta_lib = rust_record("crates/beta/src/lib.rs", "hbl1", vec![]);
    let beta_tests = rust_record(
        "crates/beta/src/tests.rs",
        "hbts1",
        vec![import_evidence(0, "use super::*;")],
    );

    h.publish(
        &[
            alpha_toml.clone(),
            alpha_lib.clone(),
            alpha_foo.clone(),
            alpha_unrelated_tests.clone(),
            beta_toml.clone(),
            beta_lib.clone(),
            beta_tests.clone(),
        ],
        false,
    )?;
    let beta_ids_before = h.edge_row_ids_for("crates/beta/src/tests.rs")?;
    let unrelated_ids_before = h.edge_row_ids_for("crates/alpha/src/unrelated/tests.rs")?;

    let alpha_bar = rust_record("crates/alpha/src/bar.rs", "hab1", vec![]);
    h.publish(std::slice::from_ref(&alpha_bar), true)?;

    ensure_eq(
        &h.edge_row_ids_for("crates/beta/src/tests.rs")?,
        &beta_ids_before,
        "adding crates/alpha/src/bar.rs does not rewrite super::* edges in unrelated crate beta",
    )?;
    ensure_eq(
        &h.edge_row_ids_for("crates/alpha/src/unrelated/tests.rs")?,
        &unrelated_ids_before,
        "adding crates/alpha/src/bar.rs does not rewrite super::* edges in unrelated subdirectory",
    )?;
    assert_cold_parity(
        &h,
        &[
            alpha_toml,
            alpha_lib,
            alpha_foo,
            alpha_unrelated_tests,
            beta_toml,
            beta_lib,
            beta_tests,
            alpha_bar,
        ],
        "scoped super re-resolution still updates matching super::bar import to match cold publish",
    )
}
