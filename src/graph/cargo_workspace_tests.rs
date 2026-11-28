use super::*;
use crate::graph::test_support::paths;
use slugaudit_test_check::{TestResult, ensure, ensure_eq};

fn assert_ws_crates(ws: &WorkspaceCrates, expected: &[(&str, Option<&str>)]) -> TestResult {
    for &(crate_ident, expected_path) in expected {
        ensure_eq(
            &ws.get(crate_ident).map(String::as_str),
            &expected_path,
            format!("workspace crate lookup for {crate_ident}"),
        )?;
    }
    Ok(())
}

fn nested_workspace_crates(
    root_manifest: &str,
    fuzz_manifest: &str,
    extra_manifests: &[(&str, &str)],
) -> WorkspaceCrates {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "crates/app/Cargo.toml",
        "crates/app/src/lib.rs",
        "fuzz/Cargo.toml",
        "fuzz/src/lib.rs",
        "fuzz/targets/one/Cargo.toml",
        "fuzz/targets/one/src/lib.rs",
        "fuzz/ignored/Cargo.toml",
        "fuzz/ignored/src/lib.rs",
        "fuzz_extra/Cargo.toml",
        "fuzz_extra/src/lib.rs",
    ]);
    let mut manifests = vec![
        ("Cargo.toml", root_manifest),
        ("crates/app/Cargo.toml", "[package]\nname = \"app-pkg\"\n"),
        ("fuzz/Cargo.toml", fuzz_manifest),
        (
            "fuzz/targets/one/Cargo.toml",
            "[package]\nname = \"fuzz-one\"\n",
        ),
        (
            "fuzz/ignored/Cargo.toml",
            "[package]\nname = \"fuzz-ignored\"\n",
        ),
    ];
    manifests.extend_from_slice(extra_manifests);
    build_workspace_crates(manifests, &known)
}

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
    assert_ws_crates(
        &ws_crates,
        &[
            ("slugaudit_test_check", Some("crates/test_check/src/lib.rs")),
            ("custom_pkg", Some("crates/custom/lib/entry.rs")),
            ("slugaudit_gates", None),
        ],
    )
}

#[test]
fn build_workspace_crates_honors_members_and_exclude_globs() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/included/Cargo.toml",
        "crates/included/src/lib.rs",
        "crates/ignored_one/Cargo.toml",
        "crates/ignored_one/src/lib.rs",
        "fuzz/Cargo.toml",
        "fuzz/src/lib.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [
            (
                "Cargo.toml",
                "[package]\nname = \"root-pkg\"\n[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"fuzz\", \"crates/ignored*\"]\n",
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

    assert_ws_crates(
        &ws_crates,
        &[
            ("included_pkg", Some("crates/included/src/lib.rs")),
            ("ignored_pkg", None),
            ("fuzz_pkg", None),
        ],
    )
}

#[test]
fn double_star_member_glob_requires_segment_boundaries() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "a/b/Cargo.toml",
        "a/b/src/lib.rs",
        "a/mid/b/Cargo.toml",
        "a/mid/b/src/lib.rs",
        "a/xb/Cargo.toml",
        "a/xb/src/lib.rs",
        "ax/b/Cargo.toml",
        "ax/b/src/lib.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [
            ("Cargo.toml", "[workspace]\nmembers = [\"a/**/b\"]\n"),
            ("a/b/Cargo.toml", "[package]\nname = \"ab-pkg\"\n"),
            ("a/mid/b/Cargo.toml", "[package]\nname = \"amidb-pkg\"\n"),
            ("a/xb/Cargo.toml", "[package]\nname = \"axb-pkg\"\n"),
            ("ax/b/Cargo.toml", "[package]\nname = \"ax-b-pkg\"\n"),
        ],
        &known,
    );

    assert_ws_crates(
        &ws_crates,
        &[
            ("ab_pkg", Some("a/b/src/lib.rs")),
            ("amidb_pkg", Some("a/mid/b/src/lib.rs")),
            ("axb_pkg", None),
            ("ax_b_pkg", None),
        ],
    )
}

#[test]
fn nested_standalone_workspace_keeps_root_and_member_crates() -> TestResult {
    let root_standalone = nested_workspace_crates(
        "[package]\nname = \"root-pkg\"\n",
        "[package]\nname = \"fuzz-pkg\"\n[workspace]\nmembers = [\"targets/*\"]\nexclude = [\"ignored\"]\n",
        &[],
    );
    assert_ws_crates(
        &root_standalone,
        &[
            ("root_pkg", Some("src/lib.rs")),
            ("app_pkg", Some("crates/app/src/lib.rs")),
            ("fuzz_pkg", Some("fuzz/src/lib.rs")),
            ("fuzz_one", Some("fuzz/targets/one/src/lib.rs")),
            ("fuzz_ignored", None),
        ],
    )
}

#[test]
fn nested_workspace_exclude_rule_leaves_prefix_sibling_crates_intact() -> TestResult {
    let both_workspaces = nested_workspace_crates(
        "[package]\nname = \"root-pkg\"\n[workspace]\nmembers = [\"crates/*\", \"fuzz_extra\"]\nexclude = [\"fuzz\"]\n",
        "[package]\nname = \"fuzz-pkg\"\n[workspace]\nmembers = [\"targets/*\"]\n",
        &[(
            "fuzz_extra/Cargo.toml",
            "[package]\nname = \"fuzz-extra\"\n",
        )],
    );
    assert_ws_crates(
        &both_workspaces,
        &[
            ("app_pkg", Some("crates/app/src/lib.rs")),
            ("fuzz_extra", Some("fuzz_extra/src/lib.rs")),
            ("fuzz_one", Some("fuzz/targets/one/src/lib.rs")),
            ("fuzz_ignored", None),
        ],
    )
}

#[test]
fn explicit_test_example_and_bench_paths_register_as_declared_crate_roots() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "custom_tests/it_entry.rs",
        "custom_tests/helper.rs",
        "custom_examples/demo_entry.rs",
        "custom_examples/support.rs",
        "custom_benches/bench_entry.rs",
        "custom_benches/common.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [(
            "Cargo.toml",
            "[package]\nname = \"pkg\"\n\
             [[test]]\nname = \"it\"\npath = \"custom_tests/it_entry.rs\"\n\
             [[example]]\nname = \"demo\"\npath = \"custom_examples/demo_entry.rs\"\n\
             [[bench]]\nname = \"perf\"\npath = \"custom_benches/bench_entry.rs\"\n",
        )],
        &known,
    );

    for target in [
        "custom_tests/it_entry.rs",
        "custom_examples/demo_entry.rs",
        "custom_benches/bench_entry.rs",
    ] {
        ensure(
            ws_crates.is_declared_crate_root(target),
            format!("{target} must be a declared crate root"),
        )?;
    }
    Ok(())
}

#[test]
fn crate_roots_in_dir_lists_explicit_test_targets() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "custom_tests/it_entry.rs",
        "custom_tests/helper.rs",
        "tests/common/mod.rs",
        "tests/group/it.rs",
        "tests/group/helper.rs",
    ]);
    let manifest = concat!(
        "[package]\nname = \"pkg\"\n",
        "[[test]]\nname = \"it_custom\"\npath = \"custom_tests/it_entry.rs\"\n",
        "[[test]]\nname = \"it_group\"\npath = \"tests/group/it.rs\"\n",
    );
    let ws = build_workspace_crates([("Cargo.toml", manifest)], &known);
    ensure_eq(
        &ws.crate_roots_in_dir("custom_tests"),
        &vec!["custom_tests/it_entry.rs"],
        "custom_tests root",
    )?;
    ensure_eq(
        &ws.crate_roots_in_dir("tests/group"),
        &vec!["tests/group/it.rs"],
        "tests/group root",
    )?;
    Ok(())
}

#[test]
fn resolve_crate_ident_supports_direct_renamed_dependency_and_scopes_to_package() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/core-api/Cargo.toml",
        "crates/core-api/src/lib.rs",
        "crates/app/Cargo.toml",
        "crates/app/src/lib.rs",
        "crates/other/Cargo.toml",
        "crates/other/src/lib.rs",
    ]);
    let ws = build_workspace_crates(
        [
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            (
                "crates/core-api/Cargo.toml",
                "[package]\nname = \"core-api\"\n",
            ),
            (
                "crates/app/Cargo.toml",
                "[package]\nname = \"app\"\n[dependencies]\napi_v2 = { package = \"core-api\", path = \"../core-api\" }\n",
            ),
            ("crates/other/Cargo.toml", "[package]\nname = \"other\"\n"),
        ],
        &known,
    );

    ensure(
        ws.has_crate_or_alias_ident("api_v2"),
        "renamed alias is tracked by has_crate_or_alias_ident",
    )?;
    ensure_eq(
        &ws.resolve_crate_ident("crates/app/src/lib.rs", "api_v2")
            .map(String::as_str),
        &Some("crates/core-api/src/lib.rs"),
        "app crate resolves api_v2 alias",
    )?;
    ensure_eq(
        &ws.resolve_crate_ident("crates/other/src/lib.rs", "api_v2")
            .map(String::as_str),
        &None,
        "sibling crate without alias does not resolve api_v2",
    )?;
    Ok(())
}

#[test]
fn resolve_crate_ident_supports_workspace_dependencies_inheritance_and_package_rename() -> TestResult
{
    let known = paths(&[
        "Cargo.toml",
        "crates/core-api/Cargo.toml",
        "crates/core-api/src/lib.rs",
        "crates/app/Cargo.toml",
        "crates/app/src/lib.rs",
        "crates/custom/Cargo.toml",
        "crates/custom/src/lib.rs",
        "crates/other/Cargo.toml",
        "crates/other/src/lib.rs",
    ]);
    let ws = build_workspace_crates(
        [
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\n\
                 [workspace.dependencies]\n\
                 api-v2 = { package = \"core-api\", path = \"crates/core-api\" }\n",
            ),
            (
                "crates/core-api/Cargo.toml",
                "[package]\nname = \"core-api\"\n",
            ),
            (
                "crates/app/Cargo.toml",
                "[package]\nname = \"app\"\n[dependencies]\napi-v2 = { workspace = true }\n",
            ),
            (
                "crates/custom/Cargo.toml",
                "[package]\nname = \"custom\"\n[dependencies]\ncustom_api = { package = \"api-v2\", workspace = true }\n",
            ),
            ("crates/other/Cargo.toml", "[package]\nname = \"other\"\n"),
        ],
        &known,
    );

    ensure_eq(
        &ws.resolve_crate_ident("crates/app/src/lib.rs", "api_v2")
            .map(String::as_str),
        &Some("crates/core-api/src/lib.rs"),
        "inherited workspace dep resolves under normalized alias",
    )?;
    ensure_eq(
        &ws.resolve_crate_ident("crates/custom/src/lib.rs", "custom_api")
            .map(String::as_str),
        &Some("crates/core-api/src/lib.rs"),
        "renamed inherited workspace dep resolves under custom_api",
    )?;
    ensure_eq(
        &ws.resolve_crate_ident("crates/other/src/lib.rs", "custom_api")
            .map(String::as_str),
        &None,
        "other crate without workspace inheritance does not resolve custom_api",
    )?;
    Ok(())
}

#[test]
fn resolve_crate_ident_supports_target_cfg_dependencies_and_lib_name_fallback() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/core-api/Cargo.toml",
        "crates/core-api/src/lib.rs",
        "crates/app/Cargo.toml",
        "crates/app/src/lib.rs",
    ]);
    let ws = build_workspace_crates(
        [
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            (
                "crates/core-api/Cargo.toml",
                "[package]\nname = \"core-api\"\n[lib]\nname = \"core_lib\"\n",
            ),
            (
                "crates/app/Cargo.toml",
                "[package]\nname = \"app\"\n\
                 [dependencies]\n\
                 pkg_alias = { package = \"core-api\" }\n\
                 [target.'cfg(unix)'.dependencies]\n\
                 unix_api = { package = \"core-api\", path = \"../core-api\" }\n",
            ),
        ],
        &known,
    );

    ensure_eq(
        &ws.resolve_crate_ident("crates/app/src/lib.rs", "pkg_alias")
            .map(String::as_str),
        &Some("crates/core-api/src/lib.rs"),
        "package rename without path resolves via libs_by_pkg_name when [lib].name differs",
    )?;
    ensure_eq(
        &ws.resolve_crate_ident("crates/app/src/lib.rs", "unix_api")
            .map(String::as_str),
        &Some("crates/core-api/src/lib.rs"),
        "target cfg renamed dependency resolves",
    )?;
    Ok(())
}
