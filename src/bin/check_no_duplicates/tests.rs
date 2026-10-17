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
    let tmp = tempfile_or_panic_path();
    let dir = tmp.join("src");
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
    let map = test_names_by_file(&[p]);
    assert!(map.contains_key("my_test"), "missing key in {map:?}");
}

fn tempfile_or_panic_path() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "slugaudit-check-no-duplicates-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("temp dir creates");
    p
}
