use super::*;

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
fn block_line_skips_comments_and_attrs() {
    // Build a synthetic file by running test_names_by_file against
    // a tiny tempdir.
    let tmp = tempfile::tempdir().expect("temp dir creates");
    let dir = tmp.path().join("src");
    std::fs::create_dir_all(&dir).expect("temp src dir creates");
    let p = dir.join("a_tests.rs");
    std::fs::write(
        &p,
        "#[test]\n\
         // comment between attr and fn\n\
         #[allow(unused)]\n\
         fn my_test() {}\n",
    )
    .expect("fixture file writes");
    let mut failures = Vec::new();
    let map = test_names_by_file(&[p], &mut failures);
    assert!(map.contains_key("my_test"), "missing key in {map:?}");
    assert!(failures.is_empty(), "unexpected: {failures:?}");
}

#[test]
fn an_unreadable_test_file_is_a_failure_not_a_skip() {
    let tmp = tempfile::tempdir().expect("temp dir creates");
    let p = tmp.path().join("invalid_utf8_tests.rs");
    std::fs::write(&p, [0xff, 0xfe, 0x00]).expect("fixture file writes");
    let mut failures = Vec::new();

    let map = test_names_by_file(&[p], &mut failures);

    assert!(map.is_empty());
    assert!(
        failures.iter().any(|f| f.contains("cannot read")),
        "{failures:?}"
    );
}

#[test]
fn an_unreadable_directory_is_a_failure_not_a_skip() {
    let tmp = tempfile::tempdir().expect("temp dir creates");
    let not_a_dir = tmp.path().join("plain_file");
    std::fs::write(&not_a_dir, "x").expect("fixture file writes");
    let mut out = Vec::new();
    let mut failures = Vec::new();

    walk_rs(&not_a_dir, &mut out, None, &mut failures);

    assert!(out.is_empty());
    assert!(
        failures.iter().any(|f| f.contains("cannot read directory")),
        "{failures:?}"
    );
}
