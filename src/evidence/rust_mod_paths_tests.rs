use crate::evidence::extract;
use crate::evidence::tests::field;
use crate::model::{EvidenceItem, EvidenceKind};
use serde_json::json;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

fn rust_module_items(source: &str) -> TestResult<Vec<EvidenceItem>> {
    let (items, truncated) = extract("rust", source).check("extract rust evidence")?;
    ensure(!truncated, "fixture extraction should not be truncated")?;
    Ok(items
        .into_iter()
        .filter(|item| {
            item.kind == EvidenceKind::Structure
                && field(&item.payload, "kind").as_str() == Some("Module")
        })
        .collect())
}

fn find_module<'a>(modules: &'a [EvidenceItem], name: &str) -> TestResult<&'a EvidenceItem> {
    modules
        .iter()
        .find(|item| field(&item.payload, "name").as_str() == Some(name))
        .check(&format!("module `{name}` present"))
}

#[test]
fn rust_mod_path_extracts_top_level_external_module_attribute() -> TestResult {
    let source = "#[path = \"custom/foo.rs\"]\npub mod foo;\n";
    let modules = rust_module_items(source)?;
    let foo = find_module(&modules, "foo")?;
    ensure_eq(
        field(&foo.payload, "rust_mod_path"),
        &json!({
            "path_attr": "custom/foo.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
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
        field(&tests_mod.payload, "rust_mod_path"),
        &json!({
            "path_attr": "foo_tests.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
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
        field(&tests_mod.payload, "rust_mod_path"),
        &json!({
            "path_attr": "unix_tests.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
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
        field(&find_module(&modules, "escaped")?.payload, "rust_mod_path"),
        &json!({
            "path_attr": "a\\b\"c/d.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
        "escaped string literal unquoted",
    )?;
    ensure_eq(
        field(
            &find_module(&modules, "raw_plain")?.payload,
            "rust_mod_path",
        ),
        &json!({
            "path_attr": "raw/sub.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
        "raw string literal unquoted",
    )?;
    ensure_eq(
        field(
            &find_module(&modules, "raw_hashed")?.payload,
            "rust_mod_path",
        ),
        &json!({
            "path_attr": "raw/\"quoted\".rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
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
    ensure(
        field(&find_module(&modules, "outer")?.payload, "rust_mod_path").is_null(),
        "inline outer mod itself has no rust_mod_path",
    )?;
    ensure(
        field(&find_module(&modules, "r#match")?.payload, "rust_mod_path").is_null(),
        "inline raw-ident mod itself has no rust_mod_path",
    )?;
    ensure(
        field(
            &find_module(&modules, "custom_inline")?.payload,
            "rust_mod_path",
        )
        .is_null(),
        "inline mod with #[path] itself has no rust_mod_path",
    )?;
    ensure_eq(
        field(&find_module(&modules, "leaf")?.payload, "rust_mod_path"),
        &json!({
            "path_attr": "leaf.rs",
            "inline_dir": "outer/match",
            "inline_depth": 2,
        }),
        "nested inline ancestors joined with raw prefix stripped",
    )?;
    ensure_eq(
        field(&find_module(&modules, "inner")?.payload, "rust_mod_path"),
        &json!({
            "path_attr": "inner.rs",
            "inline_dir": "custom_root",
            "inline_depth": 1,
        }),
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
    for item in &items {
        ensure(
            field(&item.payload, "rust_mod_path").is_null(),
            format!("no item should get rust_mod_path, got: {:?}", item.payload),
        )?;
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
        .filter(|item| !field(&item.payload, "rust_mod_path").is_null())
        .collect();
    ensure_eq(
        &with_path.len(),
        &1,
        "only external inner dup gets rust_mod_path",
    )?;
    let inner = with_path.first().check("inner dup exists")?;
    ensure_eq(
        field(&inner.payload, "rust_mod_path"),
        &json!({
            "path_attr": "inner_dup.rs",
            "inline_dir": "dup",
            "inline_depth": 1,
        }),
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
        field(&kw.payload, "rust_mod_path"),
        &json!({
            "path_attr": "kw_type.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
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
        field(&nested.payload, "rust_mod_path"),
        &json!({
            "path_attr": "nested_comment.rs",
            "inline_dir": "",
            "inline_depth": 0,
        }),
        "nested block comment inside cfg_attr does not terminate attribute parsing early",
    )?;
    Ok(())
}
