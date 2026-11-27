use super::*;
use crate::graph::test_support::paths;
use crate::graph::{RustModuleParents, resolve_imports_full};
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn parse_cargo_package_name_ignores_comments_and_non_package_tables() -> TestResult {
    let manifest = "\
        # name = \"commented-out\"\n\
        [workspace]\n\
        members = [\"crates/a\"]\n\
        \n\
        [package.metadata.deb]\n\
        name = \"wrong-metadata-name\"\n\
        \n\
        [package]\n\
        description = \"\"\"\n\
        [dependencies]\n\
        name = \"inside-multiline-string\"\n\
        \"\"\"\n\
        name = \"slugaudit-test-check\" # inline comment\n\
        \n\
        [dependencies]\n\
        name = \"wrong-dep\"\n";
    ensure_eq(
        &parse_cargo_package_name(manifest),
        &Some("slugaudit_test_check".to_owned()),
        "extracts [package] name and normalizes hyphens to underscores",
    )?;
    ensure_eq(
        &parse_cargo_package_name("[workspace]\nmembers = []\n"),
        &None,
        "virtual workspace manifest without [package] returns None",
    )?;
    Ok(())
}

#[test]
fn build_workspace_crates_maps_default_and_custom_libs_and_skips_bin_only() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "crates/test_check/Cargo.toml",
        "crates/test_check/src/lib.rs",
        "crates/custom/Cargo.toml",
        "crates/custom/lib/entry.rs",
        "crates/gates/Cargo.toml",
    ]);

    let ws_crates = build_workspace_crates(
        [
            ("Cargo.toml", "[package]\nname = \"slugaudit-mcp-rust\"\n"),
            (
                "crates/test_check/Cargo.toml",
                "[package]\nname = \"slugaudit-test-check\"\n",
            ),
            (
                "crates/custom/Cargo.toml",
                "[package]\nname = \"custom-pkg\"\n[lib]\npath = \"lib/entry.rs\"\n",
            ),
            (
                "crates/gates/Cargo.toml",
                "[package]\nname = \"slugaudit-gates\"\n",
            ),
        ],
        &known,
    );
    ensure_eq(
        &ws_crates.get("slugaudit_test_check").map(String::as_str),
        &Some("crates/test_check/src/lib.rs"),
        "maps slugaudit_test_check to crates/test_check/src/lib.rs",
    )?;
    ensure_eq(
        &ws_crates.get("custom_pkg").map(String::as_str),
        &Some("crates/custom/lib/entry.rs"),
        "maps custom [lib] path relative to member directory",
    )?;
    ensure_eq(
        &ws_crates.get("slugaudit_gates"),
        &None,
        "omits binary-only member crate without a library entry in known_paths",
    )?;
    Ok(())
}

#[test]
fn workspace_crates_resolve_cross_crate_imports_instead_of_external() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/cli.rs",
        "crates/test_check/Cargo.toml",
        "crates/test_check/src/lib.rs",
        "crates/gates/src/bin/check_x/main.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [
            ("Cargo.toml", "[package]\nname = \"slugaudit-mcp-rust\"\n"),
            (
                "crates/test_check/Cargo.toml",
                "[package]\nname = \"slugaudit-test-check\"\n",
            ),
        ],
        &known,
    );

    let edges = resolve_imports_full(
        "rust",
        "crates/gates/src/bin/check_x/main.rs",
        &[
            "use slugaudit_test_check::Check;".to_owned(),
            "use slugaudit_mcp_rust::cli;".to_owned(),
            "use slugaudit_mcp_rust::cli::Command;".to_owned(),
            "use ::slugaudit_test_check;".to_owned(),
            "use serde::Serialize;".to_owned(),
        ],
        &known,
        &RustModuleParents::new(),
        &ws_crates,
    );

    ensure_eq(&edges.len(), &5, "five use statements produce five edges")?;
    let first = edges.first().check("edge 0")?;
    ensure_eq(
        &(
            first.resolution_kind,
            first.to_relative_path.as_deref(),
            first.confidence,
        ),
        &(
            "Resolved",
            Some("crates/test_check/src/lib.rs"),
            Some("Low"),
        ),
        "use slugaudit_test_check::Check resolves to lib.rs with Low confidence",
    )?;

    let exact_mod = edges.get(1).check("edge 1")?;
    ensure_eq(
        &(exact_mod.to_relative_path.as_deref(), exact_mod.confidence),
        &(Some("src/cli.rs"), Some("High")),
        "exact module match in workspace crate carries High confidence",
    )?;

    let shortened_mod = edges.get(2).check("edge 2")?;
    ensure_eq(
        &(
            shortened_mod.to_relative_path.as_deref(),
            shortened_mod.confidence,
        ),
        &(Some("src/cli.rs"), Some("Low")),
        "shortened item match in workspace crate carries Low confidence",
    )?;

    let global_bare = edges.get(3).check("edge 3")?;
    ensure_eq(
        &(
            global_bare.to_relative_path.as_deref(),
            global_bare.confidence,
        ),
        &(Some("crates/test_check/src/lib.rs"), Some("High")),
        "leading :: bare workspace crate import resolves to lib.rs with High confidence",
    )?;

    let third_party = edges.get(4).check("edge 4")?;
    ensure_eq(
        &third_party.resolution_kind,
        &"External",
        "non-workspace crate serde stays External",
    )?;
    Ok(())
}

#[test]
fn build_workspace_crates_honors_members_exclude_globs_and_bin_entries() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/included/Cargo.toml",
        "crates/included/src/lib.rs",
        "crates/ignored_one/Cargo.toml",
        "crates/ignored_one/src/lib.rs",
        "fuzz/Cargo.toml",
        "fuzz/src/lib.rs",
        "bins/cli_entry.rs",
        "bins/sub.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [
            (
                "Cargo.toml",
                "[package]\nname = \"root-pkg\"\n[[bin]]\nname = \"cli\"\npath = \"bins/cli_entry.rs\"\n[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"fuzz\", \"crates/ignored*\"]\n",
            ),
            (
                "crates/included/Cargo.toml",
                "[package]\nname = \"included-pkg\"\n",
            ),
            (
                "crates/ignored_one/Cargo.toml",
                "[package]\nname = \"ignored-pkg\"\n",
            ),
            ("fuzz/Cargo.toml", "[package]\nname = \"fuzz-pkg\"\n"),
        ],
        &known,
    );

    ensure_eq(
        &ws_crates.get("included_pkg").map(String::as_str),
        &Some("crates/included/src/lib.rs"),
        "included member glob registers crates/included",
    )?;
    ensure_eq(
        &ws_crates.get("ignored_pkg"),
        &None,
        "exclude glob filters crates/ignored_one",
    )?;
    ensure_eq(
        &ws_crates.get("fuzz_pkg"),
        &None,
        "excluded fuzz crate is omitted from workspace libs",
    )?;

    let edges = resolve_imports_full(
        "rust",
        "bins/cli_entry.rs",
        &[
            "use crate::sub;".to_owned(),
            "use crate::RootFlag;".to_owned(),
        ],
        &known,
        &RustModuleParents::new(),
        &ws_crates,
    );
    ensure_eq(
        &edges.len(),
        &2,
        "declared [[bin]] emits two resolved edges",
    )?;
    let bin_sub = edges.first().check("bin edge 0")?;
    ensure_eq(
        &(bin_sub.to_relative_path.as_deref(), bin_sub.confidence),
        &(Some("bins/sub.rs"), Some("High")),
        "declared [[bin]] path resolves sibling module under bins/",
    )?;
    let bin_root = edges.get(1).check("bin edge 1")?;
    ensure_eq(
        &(bin_root.to_relative_path.as_deref(), bin_root.confidence),
        &(Some("bins/cli_entry.rs"), Some("Low")),
        "declared [[bin]] path anchors crate:: items to bins/cli_entry.rs",
    )?;
    Ok(())
}
