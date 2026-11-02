use super::*;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn strip_test_attr_accepts_bare_test() {
    assert_eq!(strip_test_attr("#[test]"), Some(""));
    assert_eq!(strip_test_attr("  #[test]"), Some(""));
    assert_eq!(strip_test_attr("#[test] rest"), Some(" rest"));
}

#[test]
fn strip_test_attr_accepts_qualified() {
    assert_eq!(strip_test_attr("#[tokio::test]"), Some(""));
    assert_eq!(strip_test_attr("#[some::path::test]"), Some(""));
}

#[test]
fn strip_test_attr_rejects_cfg_test() {
    assert_eq!(strip_test_attr("#[cfg(test)]"), None);
}

#[test]
fn strip_test_attr_rejects_test_case() {
    // `test_case` ends with `case`, not `test`.
    assert_eq!(strip_test_attr("#[test_case]"), None);
}

#[test]
fn same_line_fn_name_basic() {
    assert_eq!(same_line_fn_name(" fn foo() {"), Some("foo"));
}

#[test]
fn same_line_fn_name_with_pub() {
    assert_eq!(same_line_fn_name(" pub fn bar() {"), Some("bar"));
    assert_eq!(same_line_fn_name("async fn baz()"), Some("baz"));
}

#[test]
fn same_line_fn_name_skips_visibility_and_qualifiers() {
    for (after_attr, expected) in [
        (" pub async fn a()", "a"),
        (" pub(crate) fn b()", "b"),
        (" pub(in crate::x) fn c()", "c"),
        (" const fn d()", "d"),
        (" unsafe fn e()", "e"),
        (" pub const unsafe fn f()", "f"),
        (" async unsafe fn g()", "g"),
    ] {
        assert_eq!(
            same_line_fn_name(after_attr),
            Some(expected),
            "{after_attr}"
        );
    }
}

#[test]
fn same_line_fn_name_rejects_what_is_not_a_fn_item() {
    for after_attr in [
        " fnord()",
        " pubfn x()",
        " struct S;",
        " fn (x)",
        " pub(crate",
        "",
    ] {
        assert_eq!(same_line_fn_name(after_attr), None, "{after_attr:?}");
    }
}

#[test]
fn a_name_defined_twice_in_one_file_is_one_file() -> TestResult {
    let tmp = tempfile::tempdir().check("temp dir creates")?;
    let p = tmp.path().join("a_tests.rs");
    std::fs::write(
        &p,
        "#[test]\nfn dup() {}\nmod m {\n    #[test]\n    fn dup() {}\n}\n",
    )
    .check("fixture file writes")?;
    let mut failures = Vec::new();

    let map = test_names_by_file(&[p], &mut failures);

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(map.get("dup").map(BTreeSet::len), Some(1), "{map:?}");
    Ok(())
}

#[test]
fn a_qualified_test_fn_defined_in_two_files_is_found_in_both() -> TestResult {
    let tmp = tempfile::tempdir().check("temp dir creates")?;
    let a = tmp.path().join("a_tests.rs");
    let b = tmp.path().join("b_tests.rs");
    std::fs::write(&a, "#[test]\npub(crate) fn shared() {}\n").check("fixture a writes")?;
    std::fs::write(&b, "#[test]\nconst fn shared() {}\n").check("fixture b writes")?;
    let mut failures = Vec::new();

    let map = test_names_by_file(&[a, b], &mut failures);

    assert_eq!(map.get("shared").map(BTreeSet::len), Some(2), "{map:?}");
    Ok(())
}

#[test]
fn block_line_skips_comments_and_attrs() -> TestResult {
    // Build a synthetic file by running test_names_by_file against
    // a tiny tempdir.
    let tmp = tempfile::tempdir().check("temp dir creates")?;
    let dir = tmp.path().join("src");
    std::fs::create_dir_all(&dir).check("temp src dir creates")?;
    let p = dir.join("a_tests.rs");
    std::fs::write(
        &p,
        "#[test]\n\
         // comment between attr and fn\n\
         #[allow(unused)]\n\
         fn my_test() {}\n",
    )
    .check("fixture file writes")?;
    let mut failures = Vec::new();
    let map = test_names_by_file(&[p], &mut failures);
    assert!(map.contains_key("my_test"), "missing key in {map:?}");
    assert!(failures.is_empty(), "unexpected: {failures:?}");
    Ok(())
}

#[test]
fn an_unreadable_test_file_is_a_failure_not_a_skip() -> TestResult {
    let tmp = tempfile::tempdir().check("temp dir creates")?;
    let p = tmp.path().join("invalid_utf8_tests.rs");
    std::fs::write(&p, [0xff, 0xfe, 0x00]).check("fixture file writes")?;
    let mut failures = Vec::new();

    let map = test_names_by_file(&[p], &mut failures);

    assert!(map.is_empty());
    assert!(
        failures.iter().any(|f| f.contains("cannot read")),
        "{failures:?}"
    );
    Ok(())
}

#[test]
fn an_unreadable_directory_is_a_failure_not_a_skip() -> TestResult {
    let tmp = tempfile::tempdir().check("temp dir creates")?;
    let not_a_dir = tmp.path().join("plain_file");
    std::fs::write(&not_a_dir, "x").check("fixture file writes")?;
    let mut out = Vec::new();
    let mut failures = Vec::new();

    walk_rs(&not_a_dir, &mut out, &mut failures);

    assert!(out.is_empty());
    assert!(
        failures.iter().any(|f| f.contains("cannot read directory")),
        "{failures:?}"
    );
    Ok(())
}
