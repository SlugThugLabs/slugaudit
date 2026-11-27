use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

const PIN_PREFIX: &str = "tree-sitter-language-pack = \"=";

#[test]
fn pack_version_matches_the_manifest_pin() -> TestResult {
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let pin = manifest
        .lines()
        .find(|line| line.starts_with(PIN_PREFIX))
        .check("Cargo.toml must pin tree-sitter-language-pack exactly as `=x.y.z`")?;
    let version = pin
        .strip_prefix(PIN_PREFIX)
        .and_then(|rest| rest.split_once('"'))
        .map(|(version, _after)| version)
        .check("the tree-sitter-language-pack pin must close its quoted version")?;

    ensure_eq(
        &PACK_VERSION,
        &version,
        "PACK_VERSION and the tree-sitter-language-pack pin in Cargo.toml must move together",
    )?;
    Ok(())
}

#[test]
fn only_rust_is_a_tested_language() -> TestResult {
    ensure_eq(
        &TESTED_LANGUAGES,
        &["rust"],
        "TESTED_LANGUAGES == [\"rust\"]",
    )?;
    ensure(is_tested_language("rust"), "is_tested_language(\"rust\")")?;
    ensure(
        !is_tested_language("python"),
        "!is_tested_language(\"python\")",
    )?;
    ensure(
        !is_tested_language(PROSE_LANGUAGE),
        "!is_tested_language(PROSE_LANGUAGE)",
    )?;
    Ok(())
}

#[test]
fn rust_grammar_is_statically_bundled() -> TestResult {
    ensure(
        tree_sitter_language_pack::has_parser("rust"),
        "Rust parser must be statically linked and available without runtime download",
    )?;
    Ok(())
}

#[test]
fn pack_aliases_resolve_to_known_pack_languages() -> TestResult {
    for &(alias, canonical) in PACK_ALIASES {
        ensure(language_available(alias), alias)?;
        ensure(language_available(canonical), canonical)?;
    }
    Ok(())
}

#[test]
fn prose_extensions_classify_as_prose_and_data_or_source_stay_parsed() -> TestResult {
    let table = [
        ("docs/a.md", "prose"),
        ("a.markdown", "prose"),
        ("a.mdx", "prose"),
        ("a.rst", "prose"),
        ("a.adoc", "prose"),
        ("a.org", "prose"),
        ("notes.txt", "prose"),
        ("NOTES.TXT", "prose"),
        ("a.text", "prose"),
        ("data.json", "json"),
        ("ci.yaml", "yaml"),
        ("Cargo.toml", "toml"),
        ("main.rs", "rust"),
    ];
    for (path, expected) in table {
        ensure_eq(&detect_language(path).as_deref(), &Some(expected), path)?;
    }
    Ok(())
}

#[test]
fn ignore_pattern_lists_are_stored_as_prose_at_any_depth() -> TestResult {
    for path in [
        ".gitignore",
        "sub/.gitignore",
        ".ignore",
        ".dockerignore",
        ".npmignore",
        ".prettierignore",
        ".eslintignore",
        ".gitattributes",
    ] {
        ensure_eq(&detect_language(path).as_deref(), &Some("prose"), path)?;
    }
    for path in ["gitignore", ".gitignore.bak", "npmignore"] {
        ensure_eq(&detect_language(path), &None, path)?;
    }
    Ok(())
}

#[test]
fn prose_language_names_include_the_shadowed_pack_grammars() -> TestResult {
    for name in ["prose", "markdown", "rst", "org", "vimdoc"] {
        ensure(is_prose_language_name(name), name)?;
    }
    for name in ["rust", "json", "yaml", "toml", "python"] {
        ensure(!is_prose_language_name(name), name)?;
    }
    Ok(())
}

#[test]
fn detects_a_pack_language_by_path() -> TestResult {
    ensure_eq(
        &detect_language("src/main.rs").as_deref(),
        &Some("rust"),
        "detect_language(\"src/main.rs\").as_deref() == Some(\"rust\")",
    )?;
    Ok(())
}
