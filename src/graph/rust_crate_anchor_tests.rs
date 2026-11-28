use crate::graph::resolve_rust::RustResolver;
use crate::graph::resolver::LanguageResolver;
use crate::graph::test_support::paths;
use crate::graph::{
    ImportReference, RustModuleParents, build_workspace_crates, resolve_rust_mod_declaration,
};
use slugaudit_test_check::{TestResult, ensure_eq};

fn refer(text: &str) -> ImportReference {
    ImportReference {
        text: text.to_owned(),
    }
}

#[test]
fn rust_crate_anchors_build_rs_at_package_root_not_build_subdir() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "build.rs",
        "codegen.rs",
        "build/codegen.rs",
        "crates/sub/Cargo.toml",
        "crates/sub/build.rs",
        "crates/sub/gen_mod.rs",
        "crates/sub/build/gen_mod.rs",
        "crates/sub/src/gen_mod.rs",
    ]);

    let build_sub = RustResolver.resolve(&refer("crate::codegen::emit"), "build.rs", &known);
    ensure_eq(
        &(build_sub.to_relative_path.as_deref(), build_sub.confidence),
        &(Some("codegen.rs"), Some("Low")),
        "crate::codegen::emit inside build.rs resolves to package-root codegen.rs, not build/codegen.rs",
    )?;

    let build_mod = resolve_rust_mod_declaration(
        "build.rs",
        "codegen",
        None,
        &[],
        &known,
        &crate::graph::WorkspaceCrates::new(),
    );
    ensure_eq(
        &build_mod.to_relative_path.as_deref(),
        &Some("codegen.rs"),
        "mod codegen; inside build.rs resolves to package-root codegen.rs",
    )?;

    let build_root = RustResolver.resolve(&refer("crate::OUT_CONST"), "build.rs", &known);
    ensure_eq(
        &(
            build_root.to_relative_path.as_deref(),
            build_root.confidence,
        ),
        &(Some("build.rs"), Some("Low")),
        "crate:: item inside build.rs anchors at build.rs",
    )?;

    let ws_build = RustResolver.resolve(&refer("crate::gen_mod"), "crates/sub/build.rs", &known);
    ensure_eq(
        &(ws_build.to_relative_path.as_deref(), ws_build.confidence),
        &(Some("crates/sub/gen_mod.rs"), Some("High")),
        "workspace member build.rs resolves modules under crates/sub/, not crates/sub/build/",
    )?;
    Ok(())
}

#[test]
fn rust_crate_anchors_benches_tests_and_examples_in_parent_target_dir() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "benches/discovery.rs",
        "benches/common/mod.rs",
        "tests/integration.rs",
        "tests/helper.rs",
        "examples/demo.rs",
        "examples/support.rs",
        "tests/suite/main.rs",
        "tests/suite/fixture.rs",
    ]);

    let bench_crate =
        RustResolver.resolve(&refer("crate::common::foo"), "benches/discovery.rs", &known);
    ensure_eq(
        &(
            bench_crate.to_relative_path.as_deref(),
            bench_crate.confidence,
        ),
        &(Some("benches/common/mod.rs"), Some("Low")),
        "crate::common::foo in benches/discovery.rs resolves benches/common/mod.rs",
    )?;

    let bench_local = RustResolver.resolve(&refer("common::foo"), "benches/discovery.rs", &known);
    ensure_eq(
        &bench_local.to_relative_path.as_deref(),
        &Some("benches/common/mod.rs"),
        "use common::foo in benches/discovery.rs resolves benches/common/mod.rs",
    )?;

    let test_sub = RustResolver.resolve(
        &refer("crate::helper::setup"),
        "tests/integration.rs",
        &known,
    );
    ensure_eq(
        &test_sub.to_relative_path.as_deref(),
        &Some("tests/helper.rs"),
        "crate::helper::setup in tests/integration.rs resolves tests/helper.rs",
    )?;

    let test_root = RustResolver.resolve(
        &refer("crate::RootAssertion"),
        "tests/integration.rs",
        &known,
    );
    ensure_eq(
        &test_root.to_relative_path.as_deref(),
        &Some("tests/integration.rs"),
        "crate:: item in tests/integration.rs anchors at tests/integration.rs, not src/lib.rs",
    )?;

    let example_sub =
        RustResolver.resolve(&refer("crate::support::run"), "examples/demo.rs", &known);
    ensure_eq(
        &example_sub.to_relative_path.as_deref(),
        &Some("examples/support.rs"),
        "crate::support::run in examples/demo.rs resolves examples/support.rs",
    )?;

    let suite_sub = RustResolver.resolve(&refer("crate::fixture"), "tests/suite/main.rs", &known);
    ensure_eq(
        &(suite_sub.to_relative_path.as_deref(), suite_sub.confidence),
        &(Some("tests/suite/fixture.rs"), Some("High")),
        "multi-file tests/suite/main.rs resolves submodules under tests/suite/",
    )?;
    let suite_root =
        RustResolver.resolve(&refer("crate::SuiteCtx"), "tests/suite/fixture.rs", &known);
    ensure_eq(
        &suite_root.to_relative_path.as_deref(),
        &Some("tests/suite/main.rs"),
        "submodule inside tests/suite/ anchors crate:: back to tests/suite/main.rs",
    )?;
    Ok(())
}

#[test]
fn rust_crate_anchors_src_bin_single_file_vs_multi_file_main_rs() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/shared.rs",
        "src/cli.rs",
        "src/bin/tool.rs",
        "src/bin/shared.rs",
        "src/bin/daemon/main.rs",
        "src/bin/daemon/cli.rs",
        "crates/gates/Cargo.toml",
        "crates/gates/src/foo.rs",
        "crates/gates/src/bin/check_x/main.rs",
        "crates/gates/src/bin/check_x/foo.rs",
    ]);

    let bin_file_sub =
        RustResolver.resolve(&refer("crate::shared::Args"), "src/bin/tool.rs", &known);
    ensure_eq(
        &bin_file_sub.to_relative_path.as_deref(),
        &Some("src/bin/shared.rs"),
        "crate::shared in src/bin/tool.rs resolves to src/bin/shared.rs, not src/shared.rs",
    )?;

    let bin_file_root = RustResolver.resolve(&refer("crate::RunError"), "src/bin/tool.rs", &known);
    ensure_eq(
        &bin_file_root.to_relative_path.as_deref(),
        &Some("src/bin/tool.rs"),
        "crate:: item in src/bin/tool.rs anchors at src/bin/tool.rs",
    )?;

    let bin_dir_sub =
        RustResolver.resolve(&refer("crate::cli::Args"), "src/bin/daemon/main.rs", &known);
    ensure_eq(
        &bin_dir_sub.to_relative_path.as_deref(),
        &Some("src/bin/daemon/cli.rs"),
        "crate::cli in src/bin/daemon/main.rs resolves to src/bin/daemon/cli.rs",
    )?;

    let bin_dir_root =
        RustResolver.resolve(&refer("crate::DaemonErr"), "src/bin/daemon/cli.rs", &known);
    ensure_eq(
        &bin_dir_root.to_relative_path.as_deref(),
        &Some("src/bin/daemon/main.rs"),
        "crate:: item in src/bin/daemon/cli.rs anchors at src/bin/daemon/main.rs",
    )?;

    let ws_bin_sub = RustResolver.resolve(
        &refer("crate::foo::Bar"),
        "crates/gates/src/bin/check_x/main.rs",
        &known,
    );
    ensure_eq(
        &ws_bin_sub.to_relative_path.as_deref(),
        &Some("crates/gates/src/bin/check_x/foo.rs"),
        "crate::foo in crates/gates/src/bin/check_x/main.rs resolves to check_x/foo.rs",
    )?;
    Ok(())
}

#[test]
fn rust_crate_honors_custom_lib_path_from_cargo_toml() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/custom/Cargo.toml",
        "crates/custom/lib/custom_entry.rs",
        "crates/custom/lib/helper.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [(
            "crates/custom/Cargo.toml",
            "[package]\nname = \"custom-crate\"\n[lib]\npath = \"lib/custom_entry.rs\"\n",
        )],
        &known,
    );
    let parents = RustModuleParents::new();

    let sub = RustResolver.resolve_with_rust_context(
        &refer("crate::helper"),
        "crates/custom/lib/custom_entry.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &(sub.to_relative_path.as_deref(), sub.confidence),
        &(Some("crates/custom/lib/helper.rs"), Some("High")),
        "custom [lib].path anchors crate::helper under crates/custom/lib/",
    )?;

    let root_item = RustResolver.resolve_with_rust_context(
        &refer("crate::RootConfig"),
        "crates/custom/lib/helper.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &(root_item.to_relative_path.as_deref(), root_item.confidence),
        &(Some("crates/custom/lib/custom_entry.rs"), Some("Low")),
        "custom [lib].path anchors crate:: items back to lib/custom_entry.rs",
    )?;

    let mod_sub = resolve_rust_mod_declaration(
        "crates/custom/lib/custom_entry.rs",
        "helper",
        None,
        &[],
        &known,
        &ws_crates,
    );
    ensure_eq(
        &mod_sub.to_relative_path.as_deref(),
        &Some("crates/custom/lib/helper.rs"),
        "mod helper; in custom [lib].path resolves under crates/custom/lib/",
    )?;
    Ok(())
}

#[test]
fn dual_lib_and_main_resolves_crate_items_in_main_to_main_only() -> TestResult {
    let known = paths(&["Cargo.toml", "src/lib.rs", "src/main.rs"]);
    let ws = crate::graph::WorkspaceCrates::new();
    let parents = RustModuleParents::new();
    let main_item = RustResolver.resolve_with_rust_context(
        &refer("crate::MainHelper"),
        "src/main.rs",
        &known,
        &parents,
        &ws,
    );
    ensure_eq(
        &(main_item.to_relative_path.as_deref(), main_item.confidence),
        &(Some("src/main.rs"), Some("Low")),
        "crate:: item in src/main.rs anchors to src/main.rs with Low confidence, not src/lib.rs",
    )?;
    Ok(())
}

#[test]
fn custom_bin_submodules_anchor_to_bin_not_src_lib() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/cli.rs",
        "src/helper.rs",
        "bins/cli_entry.rs",
        "bins/sub.rs",
        "bins/nested/leaf.rs",
        "bins/other.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [(
            "Cargo.toml",
            "[package]\nname = \"app\"\n\
             [[bin]]\nname = \"cli\"\npath = \"bins/cli_entry.rs\"\n\
             [[bin]]\nname = \"in_src\"\npath = \"src/cli.rs\"\n",
        )],
        &known,
    );
    let parents = RustModuleParents::new();

    let bin_sub_root = RustResolver.resolve_with_rust_context(
        &refer("crate::RootFlag"),
        "bins/sub.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &(
            bin_sub_root.to_relative_path.as_deref(),
            bin_sub_root.confidence,
        ),
        &(Some("bins/cli_entry.rs"), Some("Low")),
        "crate:: item in bins/sub.rs anchors to bins/cli_entry.rs, not src/lib.rs",
    )?;

    let bin_nested_mod = RustResolver.resolve_with_rust_context(
        &refer("crate::other::Fn"),
        "bins/nested/leaf.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &bin_nested_mod.to_relative_path.as_deref(),
        &Some("bins/other.rs"),
        "crate::other in bins/nested/leaf.rs resolves under bins/",
    )?;

    let bin_super_root = RustResolver.resolve_with_rust_context(
        &refer("super::RootFlag"),
        "bins/sub.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &bin_super_root.to_relative_path.as_deref(),
        &Some("bins/cli_entry.rs"),
        "super::RootFlag in bins/sub.rs resolves back to bins/cli_entry.rs",
    )?;

    // A [[bin]] inside src/ must not hijack src/lib.rs submodules.
    let src_helper_root = RustResolver.resolve_with_rust_context(
        &refer("crate::LibItem"),
        "src/helper.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &src_helper_root.to_relative_path.as_deref(),
        &Some("src/lib.rs"),
        "src/helper.rs still anchors crate:: items to src/lib.rs when [[bin]] is in src/",
    )?;

    let in_src_bin = RustResolver.resolve_with_rust_context(
        &refer("crate::CliItem"),
        "src/cli.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &in_src_bin.to_relative_path.as_deref(),
        &Some("src/cli.rs"),
        "src/cli.rs itself still anchors crate:: items to src/cli.rs",
    )?;
    Ok(())
}

#[test]
fn custom_lib_path_resolves_super_back_to_lib_entry() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "crates/custom/Cargo.toml",
        "crates/custom/build.rs",
        "crates/custom/lib/entry.rs",
        "crates/custom/lib/helper.rs",
        "root_entry.rs",
        "root_helper.rs",
    ]);
    let ws_crates = build_workspace_crates(
        [
            (
                "Cargo.toml",
                "[package]\nname = \"root-pkg\"\n[lib]\npath = \"root_entry.rs\"\n",
            ),
            (
                "crates/custom/Cargo.toml",
                "[package]\nname = \"custom-pkg\"\n[lib]\npath = \"lib/entry.rs\"\n",
            ),
        ],
        &known,
    );
    let parents = RustModuleParents::new();

    let super_item = RustResolver.resolve_with_rust_context(
        &refer("super::RootItem"),
        "crates/custom/lib/helper.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &(
            super_item.to_relative_path.as_deref(),
            super_item.confidence,
        ),
        &(Some("crates/custom/lib/entry.rs"), Some("Low")),
        "super::RootItem from custom [lib].path submodule resolves to lib/entry.rs",
    )?;

    let super_glob = RustResolver.resolve_with_rust_context(
        &refer("super::*"),
        "crates/custom/lib/helper.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &super_glob.to_relative_path.as_deref(),
        &Some("crates/custom/lib/entry.rs"),
        "super::* from custom [lib].path submodule resolves to lib/entry.rs",
    )?;

    let super_build = RustResolver.resolve_with_rust_context(
        &refer("super::build"),
        "crates/custom/lib/helper.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &super_build.to_relative_path.as_deref(),
        &Some("crates/custom/lib/entry.rs"),
        "super::build stays inside custom [lib] boundary rather than climbing to build.rs",
    )?;

    let root_super = RustResolver.resolve_with_rust_context(
        &refer("super::RootItem"),
        "root_helper.rs",
        &known,
        &parents,
        &ws_crates,
    );
    ensure_eq(
        &root_super.to_relative_path.as_deref(),
        &Some("root_entry.rs"),
        "super::RootItem from root-level custom [lib].path submodule resolves to root_entry.rs",
    )?;
    Ok(())
}

#[test]
fn explicit_test_target_submodules_anchor_to_test_root_not_lib_or_shared_tests() -> TestResult {
    let known = paths(&[
        "Cargo.toml",
        "src/lib.rs",
        "src/helper.rs",
        "custom_tests/it_entry.rs",
        "custom_tests/helper.rs",
        "tests/common/mod.rs",
        "tests/helper.rs",
        "tests/group/it.rs",
        "tests/group/helper.rs",
    ]);
    let ws = build_workspace_crates(
        [(
            "Cargo.toml",
            "[package]\nname = \"app\"\n\
             [[test]]\nname = \"custom\"\npath = \"custom_tests/it_entry.rs\"\n\
             [[test]]\nname = \"group\"\npath = \"tests/group/it.rs\"\n",
        )],
        &known,
    );
    let parents = RustModuleParents::new();

    for (file, imp, expected_target, expected_conf) in [
        (
            "custom_tests/helper.rs",
            "crate::helper",
            "custom_tests/helper.rs",
            "High",
        ),
        (
            "custom_tests/helper.rs",
            "crate::RootItem",
            "custom_tests/it_entry.rs",
            "Low",
        ),
        (
            "custom_tests/helper.rs",
            "super::RootItem",
            "custom_tests/it_entry.rs",
            "Low",
        ),
        (
            "tests/group/helper.rs",
            "crate::helper",
            "tests/group/helper.rs",
            "High",
        ),
        (
            "tests/group/helper.rs",
            "crate::RootItem",
            "tests/group/it.rs",
            "Low",
        ),
        (
            "tests/group/helper.rs",
            "super::RootItem",
            "tests/group/it.rs",
            "Low",
        ),
    ] {
        let res = RustResolver.resolve_with_rust_context(&refer(imp), file, &known, &parents, &ws);
        ensure_eq(
            &(res.to_relative_path.as_deref(), res.confidence),
            &(Some(expected_target), Some(expected_conf)),
            format!("{file} resolves {imp} to {expected_target}"),
        )?;
    }
    Ok(())
}
