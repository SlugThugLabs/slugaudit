use crate::graph::resolver::resolve_one;
use crate::graph::test_support::paths;

#[test]
fn python_dot_import_resolves_to_package_init() {
    let known = paths(&["pkg/__init__.py", "pkg/a.py"]);
    let result = resolve_one("python", "pkg/a.py", ".", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("pkg/__init__.py"));
}

#[test]
fn python_single_dot_sibling_module() {
    let known = paths(&["pkg/a.py", "pkg/bar.py"]);
    let result = resolve_one("python", "pkg/a.py", ".bar", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("pkg/bar.py"));
}

#[test]
fn python_double_dot_goes_up_one_package() {
    let known = paths(&["pkg/sub/a.py", "pkg/mod.py"]);
    let result = resolve_one("python", "pkg/sub/a.py", "..mod", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("pkg/mod.py"));
}

#[test]
fn python_dotted_path_prefers_package_form() {
    let known = paths(&["pkg/a.py", "pkg/sub/__init__.py"]);
    let result = resolve_one("python", "pkg/a.py", ".sub", &known);
    assert_eq!(
        result.to_relative_path.as_deref(),
        Some("pkg/sub/__init__.py")
    );
}

#[test]
fn python_absolute_import_is_external() {
    let known = paths(&["pkg/a.py"]);
    let result = resolve_one("python", "pkg/a.py", "os", &known);
    assert_eq!(result.kind.as_sql_text(), "External");
    assert_eq!(result.to_relative_path, None);
}

#[test]
fn python_relative_import_with_no_matching_file_is_unresolved() {
    let known = paths(&["pkg/a.py"]);
    let result = resolve_one("python", "pkg/a.py", ".missing", &known);
    assert_eq!(result.kind.as_sql_text(), "Unresolved");
}

#[test]
fn js_relative_import_resolves_with_guessed_extension() {
    let known = paths(&["src/a.ts", "src/utils.ts"]);
    let result = resolve_one("javascript", "src/a.ts", "./utils", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("src/utils.ts"));
    assert_eq!(result.confidence, Some("High"));
}

#[test]
fn js_relative_import_resolves_to_directory_index() {
    let known = paths(&["src/a.ts", "src/lib/index.ts"]);
    let result = resolve_one("javascript", "src/pages/a.ts", "../lib", &known);
    assert_eq!(result.to_relative_path.as_deref(), Some("src/lib/index.ts"));
}

#[test]
fn js_ambiguous_extension_match_is_low_confidence() {
    let known = paths(&["src/a.ts", "src/utils.ts", "src/utils.js"]);
    let result = resolve_one("javascript", "src/a.ts", "./utils", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.confidence, Some("Low"));
}

#[test]
fn js_bare_package_name_is_external() {
    let known = paths(&["src/a.ts"]);
    let result = resolve_one("javascript", "src/a.ts", "react", &known);
    assert_eq!(result.kind.as_sql_text(), "External");
}

#[test]
fn an_unsupported_language_is_always_unresolved() {
    let known = paths(&["a.go"]);
    let result = resolve_one("go", "a.go", "./local", &known);
    assert_eq!(result.kind.as_sql_text(), "Unresolved");
}
