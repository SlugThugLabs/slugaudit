use super::*;
use slugaudit_test_check::{TestResult, ensure_eq};

#[test]
fn an_empty_path_is_authored() -> TestResult {
    ensure_eq(
        &classify(""),
        &FileClass::Authored,
        "classify(\"\") == FileClass::Authored",
    )?;
    Ok(())
}

#[test]
fn an_ordinary_source_path_is_authored() -> TestResult {
    ensure_eq(
        &classify("src/auth/login.rs"),
        &FileClass::Authored,
        "classify(\"src/auth/login.rs\") == FileClass::Authored",
    )?;
    ensure_eq(
        &classify("README.md"),
        &FileClass::Authored,
        "classify(\"README.md\") == FileClass::Authored",
    )?;
    ensure_eq(
        &classify("src/vendor.rs"),
        &FileClass::Authored,
        "classify(\"src/vendor.rs\") == FileClass::Authored",
    )?;
    Ok(())
}

#[test]
fn a_third_party_directory_is_vendored() -> TestResult {
    ensure_eq(
        &classify("vendor/foo/bar.go"),
        &FileClass::Vendored,
        "classify(\"vendor/foo/bar.go\") == FileClass::Vendored",
    )?;
    ensure_eq(
        &classify("node_modules/left-pad/index.js"),
        &FileClass::Vendored,
        "classify(\"node_modules/left-pad/index.js\") == FileClass::Vendored",
    )?;
    ensure_eq(
        &classify("Pods/Alamofire/Source/Alamofire.swift"),
        &FileClass::Vendored,
        "classify(\"Pods/Alamofire/Source/Alamofire.swift\") == FileClass::Vendored",
    )?;
    ensure_eq(
        &classify("site-packages/django/db/models.py"),
        &FileClass::Vendored,
        "classify(\"site-packages/django/db/models.py\") == FileClass::Vendored",
    )?;
    Ok(())
}

#[test]
fn a_generated_path_or_suffix_is_generated() -> TestResult {
    ensure_eq(
        &classify("src/generated/api.rs"),
        &FileClass::Generated,
        "classify(\"src/generated/api.rs\") == FileClass::Generated",
    )?;
    ensure_eq(
        &classify("api/service.pb.go"),
        &FileClass::Generated,
        "classify(\"api/service.pb.go\") == FileClass::Generated",
    )?;
    ensure_eq(
        &classify("web/app.min.js"),
        &FileClass::Generated,
        "classify(\"web/app.min.js\") == FileClass::Generated",
    )?;
    ensure_eq(
        &classify("zz_generated.deepcopy.go"),
        &FileClass::Generated,
        "classify(\"zz_generated.deepcopy.go\") == FileClass::Generated",
    )?;
    ensure_eq(
        &classify("models/user_pb2.py"),
        &FileClass::Generated,
        "classify(\"models/user_pb2.py\") == FileClass::Generated",
    )?;
    ensure_eq(
        &classify("ui/widget.freezed.dart"),
        &FileClass::Generated,
        "classify(\"ui/widget.freezed.dart\") == FileClass::Generated",
    )?;
    Ok(())
}

#[test]
fn a_generated_file_inside_a_vendored_tree_is_vendored() -> TestResult {
    ensure_eq(
        &classify("vendor/mod.pb.go"),
        &FileClass::Vendored,
        "classify(\"vendor/mod.pb.go\") == FileClass::Vendored",
    )?;
    ensure_eq(
        &classify("node_modules/x/generated/api.rs"),
        &FileClass::Vendored,
        "classify(\"node_modules/x/generated/api.rs\") == FileClass::Vendored",
    )?;
    Ok(())
}

#[test]
fn a_word_containing_generated_is_not_generated() -> TestResult {
    ensure_eq(
        &classify("src/regenerated.rs"),
        &FileClass::Authored,
        "classify(\"src/regenerated.rs\") == FileClass::Authored",
    )?;
    ensure_eq(
        &classify("src/ungenerated_notes.md"),
        &FileClass::Authored,
        "classify(\"src/ungenerated_notes.md\") == FileClass::Authored",
    )?;
    Ok(())
}

#[test]
fn the_sql_text_is_the_stable_stored_tag() -> TestResult {
    ensure_eq(
        &FileClass::Authored.as_sql_text(),
        &"authored",
        "FileClass::Authored.as_sql_text() == \"authored\"",
    )?;
    ensure_eq(
        &FileClass::Generated.as_sql_text(),
        &"generated",
        "FileClass::Generated.as_sql_text() == \"generated\"",
    )?;
    ensure_eq(
        &FileClass::Vendored.as_sql_text(),
        &"vendored",
        "FileClass::Vendored.as_sql_text() == \"vendored\"",
    )?;
    Ok(())
}
