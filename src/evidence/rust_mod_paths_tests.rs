use crate::evidence::extract;
use crate::model::{EvidencePayload, RustModPathPayload, StructurePayload};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

fn rust_module_items(source: &str) -> TestResult<Vec<StructurePayload>> {
    let (items, truncated) = extract("rust", source).check("extract rust evidence")?;
    ensure(!truncated, "fixture extraction should not be truncated")?;
    Ok(items
        .into_iter()
        .filter_map(|item| match item.payload {
            EvidencePayload::Structure(s) if s.kind == "Module" => Some(s),
            _ => None,
        })
        .collect())
}

fn find_module<'a>(
    modules: &'a [StructurePayload],
    name: &str,
) -> TestResult<&'a StructurePayload> {
    modules
        .iter()
        .find(|s| s.name.as_deref() == Some(name))
        .check(&format!("module `{name}` present"))
}

fn expected_path(
    path_attr: Option<&str>,
    inline_dir: &str,
    inline_segments: &[&str],
) -> RustModPathPayload {
    expected_path_with_extras(path_attr, &[], inline_dir, inline_segments)
}

fn expected_path_with_extras(
    path_attr: Option<&str>,
    extra_path_attrs: &[&str],
    inline_dir: &str,
    inline_segments: &[&str],
) -> RustModPathPayload {
    RustModPathPayload {
        path_attr: path_attr.map(str::to_owned),
        extra_path_attrs: extra_path_attrs.iter().map(|s| (*s).to_owned()).collect(),
        inline_dir: inline_dir.to_owned(),
        inline_depth: inline_segments.len(),
        inline_segments: inline_segments.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn rust_mod_path_extracts_top_level_external_module_attribute() -> TestResult {
    let source = "#[path = \"custom/foo.rs\"]\npub mod foo;\n";
    let modules = rust_module_items(source)?;
    let foo = find_module(&modules, "foo")?;
    ensure_eq(
        &foo.rust_mod_path,
        &Some(expected_path(Some("custom/foo.rs"), "", &[])),
        "top-level external mod receives rust_mod_path",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_extracts_cfg_attr_with_multiple_predicates_and_attrs() -> TestResult {
    let source = "#[cfg(test)]\n#[cfg_attr(all(test, unix), cfg(test), path = \"foo_tests.rs\")]\nmod tests;\n";
    let modules = rust_module_items(source)?;
    let tests_mod = find_module(&modules, "tests")?;
    ensure_eq(
        &tests_mod.rust_mod_path,
        &Some(expected_path(Some("foo_tests.rs"), "", &[])),
        "cfg_attr with nested predicate and multiple attrs extracts path",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_extracts_nested_cfg_attr_and_comment_separated_attr() -> TestResult {
    let source = "#[cfg_attr(unix, cfg_attr(test, path = \"unix_tests.rs\"))]\n/// doc comment between attr and mod\nmod tests;\n";
    let modules = rust_module_items(source)?;
    let tests_mod = find_module(&modules, "tests")?;
    ensure_eq(
        &tests_mod.rust_mod_path,
        &Some(expected_path(Some("unix_tests.rs"), "", &[])),
        "nested cfg_attr and doc comment before mod still attach path",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_unquotes_escaped_and_raw_string_literals() -> TestResult {
    let source = concat!(
        "#[path = \"a\\\\b\\\"c\\x2f\\u{64}.rs\"]\nmod escaped;\n",
        "#[path = r\"raw/sub.rs\"]\nmod raw_plain;\n",
        "#[path = r#\"raw/\"quoted\".rs\"#]\nmod raw_hashed;\n",
    );
    let modules = rust_module_items(source)?;
    ensure_eq(
        &find_module(&modules, "escaped")?.rust_mod_path,
        &Some(expected_path(Some("a\\b\"c/d.rs"), "", &[])),
        "escaped string literal unquoted",
    )?;
    ensure_eq(
        &find_module(&modules, "raw_plain")?.rust_mod_path,
        &Some(expected_path(Some("raw/sub.rs"), "", &[])),
        "raw string literal unquoted",
    )?;
    ensure_eq(
        &find_module(&modules, "raw_hashed")?.rust_mod_path,
        &Some(expected_path(Some("raw/\"quoted\".rs"), "", &[])),
        "raw hashed string literal unquoted",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_tracks_inline_ancestors_and_custom_inline_path() -> TestResult {
    let source = concat!(
        "mod outer {\n",
        "    mod r#match {\n",
        "        #[path = \"leaf.rs\"]\n",
        "        mod leaf;\n",
        "    }\n",
        "}\n",
        "#[path = \"custom_root\"]\n",
        "mod custom_inline {\n",
        "    #[path = \"inner.rs\"]\n",
        "    mod inner;\n",
        "}\n",
    );
    let modules = rust_module_items(source)?;
    ensure_eq(
        &find_module(&modules, "outer")?.rust_mod_path,
        &None,
        "inline outer mod itself has no rust_mod_path",
    )?;
    ensure_eq(
        &find_module(&modules, "r#match")?.rust_mod_path,
        &None,
        "inline raw-ident mod itself has no rust_mod_path",
    )?;
    ensure_eq(
        &find_module(&modules, "custom_inline")?.rust_mod_path,
        &None,
        "inline mod with #[path] itself has no rust_mod_path",
    )?;
    ensure_eq(
        &find_module(&modules, "leaf")?.rust_mod_path,
        &Some(expected_path(
            Some("leaf.rs"),
            "outer/match",
            &["outer", "match"],
        )),
        "nested inline ancestors joined with raw prefix stripped",
    )?;
    ensure_eq(
        &find_module(&modules, "inner")?.rust_mod_path,
        &Some(expected_path(
            Some("inner.rs"),
            "custom_root",
            &["custom_root"],
        )),
        "inline mod #[path] overrides ancestor directory segment",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_ignores_plain_mods_non_mods_decoys_and_non_path_attrs() -> TestResult {
    let source = concat!(
        "mod plain;\n",
        "#[doc = \"#[path = \\\"fake.rs\\\"]\"]\n",
        "#[path::to::something]\n",
        "#[foo::path = \"fake2.rs\"]\n",
        "#[cfg_attr(path = \"fake3.rs\", cfg(test))]\n",
        "mod decoys;\n",
        "#[path = \"ignored_fn.rs\"]\n",
        "fn helper() {\n",
        "    // #[path = \"comment_fake.rs\"] mod commented;\n",
        "    let s = \"#[path = \\\"str_fake.rs\\\"] mod stringed;\";\n",
        "    let _unused = s;\n",
        "}\n",
        "#[path = \"ignored_struct.rs\"]\n",
        "struct Widget;\n",
    );
    let (items, truncated) = extract("rust", source).check("extract rust evidence")?;
    ensure(!truncated, "decoys fixture should not truncate")?;
    let modules = rust_module_items(source)?;
    for name in ["plain", "decoys"] {
        ensure_eq(
            &find_module(&modules, name)?.rust_mod_path,
            &Some(expected_path(None, "", &[])),
            format!("external mod `{name}` gets None path_attr without #[path]"),
        )?;
    }
    for item in &items {
        if let EvidencePayload::Structure(s) = &item.payload
            && s.kind != "Module"
        {
            ensure_eq(
                &s.rust_mod_path,
                &None,
                format!("non-module Structure must not get rust_mod_path: {s:?}"),
            )?;
        }
    }
    Ok(())
}

#[test]
fn rust_mod_path_disambiguates_duplicate_module_names_by_span() -> TestResult {
    let source = concat!(
        "mod dup {\n",
        "    #[path = \"inner_dup.rs\"]\n",
        "    mod dup;\n",
        "}\n",
    );
    let modules = rust_module_items(source)?;
    ensure_eq(&modules.len(), &2, "two modules named dup")?;
    let with_path: Vec<_> = modules
        .iter()
        .filter(|s| s.rust_mod_path.is_some())
        .collect();
    ensure_eq(
        &with_path.len(),
        &1,
        "only external inner dup gets rust_mod_path",
    )?;
    let inner = with_path.first().check("inner dup exists")?;
    ensure_eq(
        &inner.rust_mod_path,
        &Some(expected_path(Some("inner_dup.rs"), "dup", &["dup"])),
        "inner external dup carries outer inline_dir",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_matches_raw_identifier_external_module() -> TestResult {
    let source = "#[path = \"kw_type.rs\"]\nmod r#type;\n";
    let modules = rust_module_items(source)?;
    let kw = find_module(&modules, "r#type")?;
    ensure_eq(
        &kw.rust_mod_path,
        &Some(expected_path(Some("kw_type.rs"), "", &[])),
        "external raw identifier module matched",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_handles_nested_block_comments_inside_cfg_attr() -> TestResult {
    let source =
        "#[cfg_attr(/* /* nested */ */ test, path = \"nested_comment.rs\")]\nmod nested;\n";
    let modules = rust_module_items(source)?;
    let nested = find_module(&modules, "nested")?;
    ensure_eq(
        &nested.rust_mod_path,
        &Some(expected_path(Some("nested_comment.rs"), "", &[])),
        "nested block comment inside cfg_attr handled by CST walker",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_preserves_multi_segment_inline_path_depth() -> TestResult {
    let source = concat!(
        "#[path = \"a/b\"]\n",
        "mod outer {\n",
        "    #[path = \"leaf.rs\"]\n",
        "    mod leaf;\n",
        "}\n",
    );
    let modules = rust_module_items(source)?;
    let leaf = find_module(&modules, "leaf")?;
    ensure_eq(
        &leaf.rust_mod_path,
        &Some(expected_path(Some("leaf.rs"), "a/b", &["a/b"])),
        "multi-segment inline #[path] preserves a single module-nesting segment",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_emits_metadata_for_plain_external_mod_under_inline_path() -> TestResult {
    let source = concat!(
        "#[path = \"custom_dir\"]\n",
        "mod outer {\n",
        "    mod child;\n",
        "    mod nested {\n",
        "        mod r#match;\n",
        "    }\n",
        "}\n",
        "mod plain_outer {\n",
        "    mod plain_child;\n",
        "}\n",
    );
    let modules = rust_module_items(source)?;
    ensure_eq(
        &find_module(&modules, "child")?.rust_mod_path,
        &Some(expected_path(Some("child"), "custom_dir", &["custom_dir"])),
        "plain external mod under inline #[path] receives rust_mod_path",
    )?;
    ensure_eq(
        &find_module(&modules, "r#match")?.rust_mod_path,
        &Some(expected_path(
            Some("match"),
            "custom_dir/nested",
            &["custom_dir", "nested"],
        )),
        "nested raw-ident external mod under inline #[path] strips r# in path_attr",
    )?;
    ensure_eq(
        &find_module(&modules, "plain_outer")?.rust_mod_path,
        &None,
        "inline plain_outer mod has no rust_mod_path",
    )?;
    ensure_eq(
        &find_module(&modules, "plain_child")?.rust_mod_path,
        &Some(expected_path(None, "plain_outer", &["plain_outer"])),
        "plain external mod without #[path] ancestor gets rust_mod_path with None path_attr",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_collects_multiple_cfg_attr_paths_deduped_in_source_order() -> TestResult {
    let source = concat!(
        "#[cfg_attr(target_os = \"linux\", path = \"linux.rs\")]\n",
        "#[cfg_attr(windows, path = \"windows.rs\")]\n",
        "#[cfg_attr(target_os = \"freebsd\", path = \"linux.rs\")]\n",
        "#[cfg_attr(target_os = \"macos\", path = \"macos.rs\")]\n",
        "mod os;\n",
    );
    let modules = rust_module_items(source)?;
    let os_mod = find_module(&modules, "os")?;
    ensure_eq(
        &os_mod.rust_mod_path,
        &Some(expected_path_with_extras(
            Some("linux.rs"),
            &["windows.rs", "macos.rs"],
            "",
            &[],
        )),
        "multiple cfg_attr path targets preserve first in path_attr and deduplicated rest in extra_path_attrs",
    )?;
    Ok(())
}

#[test]
fn rust_mod_path_payload_json_backwards_compatible_with_extra_path_attrs() -> TestResult {
    let legacy_json =
        r#"{"path_attr":"foo.rs","inline_dir":"","inline_depth":0,"inline_segments":[]}"#;
    let decoded: RustModPathPayload =
        serde_json::from_str(legacy_json).check("deserialize legacy RustModPathPayload JSON")?;
    ensure_eq(
        &decoded,
        &expected_path(Some("foo.rs"), "", &[]),
        "legacy JSON without extra_path_attrs deserializes with empty extra_path_attrs",
    )?;
    let encoded_single =
        serde_json::to_string(&decoded).check("serialize single-path RustModPathPayload")?;
    ensure(
        !encoded_single.contains("extra_path_attrs"),
        "empty extra_path_attrs is omitted during serialization",
    )?;

    let multi = expected_path_with_extras(Some("unix.rs"), &["win.rs"], "inner", &["inner"]);
    let encoded_multi =
        serde_json::to_string(&multi).check("serialize multi-path RustModPathPayload")?;
    let roundtrip: RustModPathPayload =
        serde_json::from_str(&encoded_multi).check("roundtrip multi-path JSON")?;
    ensure_eq(
        &roundtrip,
        &multi,
        "multi-path payload roundtrips through JSON",
    )?;
    Ok(())
}
