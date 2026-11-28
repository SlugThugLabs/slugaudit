use super::{WorkspaceCrates, build_workspace_crates};
use crate::graph::test_support::paths;
use crate::graph::{RustModuleParents, resolve_imports_full};
use slugaudit_test_check::{TestResult, ensure_eq};
use std::collections::HashSet;

fn assert_resolved_imports(
    importing_file: &str,
    known: &HashSet<&str>,
    ws_crates: &WorkspaceCrates,
    expected: &[(&str, &str, Option<&str>, Option<&str>)],
) -> TestResult {
    let raw_imports: Vec<String> = expected.iter().map(|(raw, ..)| (*raw).to_owned()).collect();
    let edges = resolve_imports_full(
        "rust",
        importing_file,
        &raw_imports,
        known,
        &RustModuleParents::new(),
        ws_crates,
    );
    ensure_eq(
        &edges.len(),
        &expected.len(),
        format!("{importing_file} edge count"),
    )?;
    for (idx, ((raw, kind, target, conf), edge)) in expected.iter().zip(edges.iter()).enumerate() {
        ensure_eq(
            &(
                edge.raw_import_text.as_str(),
                edge.resolution_kind,
                edge.to_relative_path.as_deref(),
                edge.confidence,
            ),
            &(*raw, *kind, *target, *conf),
            format!("{importing_file} edge {idx} ({raw})"),
        )?;
    }
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

    assert_resolved_imports(
        "crates/gates/src/bin/check_x/main.rs",
        &known,
        &ws_crates,
        &[
            (
                "use slugaudit_test_check::Check;",
                "Resolved",
                Some("crates/test_check/src/lib.rs"),
                Some("Low"),
            ),
            (
                "use slugaudit_mcp_rust::cli;",
                "Resolved",
                Some("src/cli.rs"),
                Some("High"),
            ),
            (
                "use slugaudit_mcp_rust::cli::Command;",
                "Resolved",
                Some("src/cli.rs"),
                Some("Low"),
            ),
            (
                "use ::slugaudit_test_check;",
                "Resolved",
                Some("crates/test_check/src/lib.rs"),
                Some("High"),
            ),
            ("use serde::Serialize;", "External", None, None),
        ],
    )
}

#[test]
fn declared_bin_target_resolves_sibling_modules_and_crate_items() -> TestResult {
    let known = paths(&["Cargo.toml", "bins/cli_entry.rs", "bins/sub.rs"]);
    let ws_crates = build_workspace_crates(
        [(
            "Cargo.toml",
            "[package]\nname = \"root-pkg\"\n[[bin]]\nname = \"cli\"\npath = \"bins/cli_entry.rs\"\n",
        )],
        &known,
    );

    assert_resolved_imports(
        "bins/cli_entry.rs",
        &known,
        &ws_crates,
        &[
            (
                "use crate::sub;",
                "Resolved",
                Some("bins/sub.rs"),
                Some("High"),
            ),
            (
                "use crate::RootFlag;",
                "Resolved",
                Some("bins/cli_entry.rs"),
                Some("Low"),
            ),
        ],
    )
}

#[test]
fn declared_test_example_and_bench_targets_resolve_self_submodules() -> TestResult {
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

    for (entry, raw_import, expected) in [
        (
            "custom_tests/it_entry.rs",
            "use self::helper;",
            "custom_tests/helper.rs",
        ),
        (
            "custom_examples/demo_entry.rs",
            "use self::support;",
            "custom_examples/support.rs",
        ),
        (
            "custom_benches/bench_entry.rs",
            "use self::common;",
            "custom_benches/common.rs",
        ),
    ] {
        assert_resolved_imports(
            entry,
            &known,
            &ws_crates,
            &[(raw_import, "Resolved", Some(expected), Some("High"))],
        )?;
    }
    Ok(())
}

#[test]
fn declared_test_roots_resolve_crate_and_super_between_entry_and_helper() -> TestResult {
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

    for (from, imp, want, conf) in [
        (
            "custom_tests/it_entry.rs",
            "use crate::helper;",
            "custom_tests/helper.rs",
            "High",
        ),
        (
            "custom_tests/helper.rs",
            "use crate::RootItem;",
            "custom_tests/it_entry.rs",
            "Low",
        ),
        (
            "custom_tests/helper.rs",
            "use super::RootItem;",
            "custom_tests/it_entry.rs",
            "Low",
        ),
        (
            "tests/group/it.rs",
            "use crate::helper;",
            "tests/group/helper.rs",
            "High",
        ),
        (
            "tests/group/helper.rs",
            "use crate::RootItem;",
            "tests/group/it.rs",
            "Low",
        ),
        (
            "tests/group/helper.rs",
            "use super::RootItem;",
            "tests/group/it.rs",
            "Low",
        ),
    ] {
        assert_resolved_imports(
            from,
            &known,
            &ws,
            &[(imp, "Resolved", Some(want), Some(conf))],
        )?;
    }
    Ok(())
}

#[test]
fn renamed_cargo_dependency_resolves_for_declaring_crate_and_stays_external_for_sibling()
-> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/core-api/Cargo.toml",
        "crates/core-api/src/lib.rs",
        "crates/core-api/src/auth.rs",
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
                "[package]\nname = \"app\"\n\
                 [dependencies]\n\
                 api_v2 = { package = \"core-api\", path = \"../core-api\" }\n",
            ),
            ("crates/other/Cargo.toml", "[package]\nname = \"other\"\n"),
        ],
        &known,
    );

    assert_resolved_imports(
        "crates/app/src/lib.rs",
        &known,
        &ws,
        &[
            (
                "use api_v2::Client;",
                "Resolved",
                Some("crates/core-api/src/lib.rs"),
                Some("Low"),
            ),
            (
                "extern crate api_v2;",
                "Resolved",
                Some("crates/core-api/src/lib.rs"),
                Some("High"),
            ),
            (
                "use api_v2::auth;",
                "Resolved",
                Some("crates/core-api/src/auth.rs"),
                Some("High"),
            ),
        ],
    )?;

    assert_resolved_imports(
        "crates/other/src/lib.rs",
        &known,
        &ws,
        &[("use api_v2::Client;", "External", None, None)],
    )
}

#[test]
fn workspace_dependencies_inheritance_resolves_renamed_and_custom_aliased_imports() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/core-api/Cargo.toml",
        "crates/core-api/src/lib.rs",
        "crates/core-api/src/auth.rs",
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
                "[package]\nname = \"custom\"\n\
                 [dependencies]\n\
                 custom_api = { package = \"api-v2\", workspace = true }\n",
            ),
            ("crates/other/Cargo.toml", "[package]\nname = \"other\"\n"),
        ],
        &known,
    );

    assert_resolved_imports(
        "crates/app/src/lib.rs",
        &known,
        &ws,
        &[
            (
                "use api_v2::Client;",
                "Resolved",
                Some("crates/core-api/src/lib.rs"),
                Some("Low"),
            ),
            (
                "use api_v2::auth;",
                "Resolved",
                Some("crates/core-api/src/auth.rs"),
                Some("High"),
            ),
        ],
    )?;
    assert_resolved_imports(
        "crates/custom/src/lib.rs",
        &known,
        &ws,
        &[(
            "use custom_api::auth;",
            "Resolved",
            Some("crates/core-api/src/auth.rs"),
            Some("High"),
        )],
    )?;
    assert_resolved_imports(
        "crates/other/src/lib.rs",
        &known,
        &ws,
        &[("use api_v2::Client;", "External", None, None)],
    )
}

#[test]
fn target_cfg_renamed_dependency_resolves_workspace_sibling_imports() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/core-api/Cargo.toml",
        "crates/core-api/src/lib.rs",
        "crates/core-api/src/auth.rs",
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
                "[package]\nname = \"app\"\n\
                 [target.'cfg(unix)'.dependencies]\n\
                 unix_api = { package = \"core-api\", path = \"../core-api\" }\n",
            ),
            ("crates/other/Cargo.toml", "[package]\nname = \"other\"\n"),
        ],
        &known,
    );

    assert_resolved_imports(
        "crates/app/src/lib.rs",
        &known,
        &ws,
        &[
            (
                "use unix_api::Client;",
                "Resolved",
                Some("crates/core-api/src/lib.rs"),
                Some("Low"),
            ),
            (
                "use unix_api::auth;",
                "Resolved",
                Some("crates/core-api/src/auth.rs"),
                Some("High"),
            ),
        ],
    )?;
    assert_resolved_imports(
        "crates/other/src/lib.rs",
        &known,
        &ws,
        &[("use unix_api::Client;", "External", None, None)],
    )
}
