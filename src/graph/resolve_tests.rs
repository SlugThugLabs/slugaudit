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

#[test]
fn kotlin_module_import_resolves_to_the_real_file() {
    let known = paths(&["util/Helper.kt", "main.kt"]);
    let result = resolve_one("kotlin", "main.kt", "import util.Helper", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("util/Helper.kt"));
}

#[test]
fn kotlin_stdlib_import_is_not_faked_resolved() {
    let known = paths(&["main.kt"]);
    let result = resolve_one("kotlin", "main.kt", "import kotlin.math.max", &known);
    // A dotted stdlib module can't be told apart from a project module
    // without a file match — the same honest Unresolved verdict python
    // gives `import os.path`. The point: never faked as Resolved.
    assert_eq!(result.kind.as_sql_text(), "Unresolved");
    assert_eq!(result.to_relative_path, None);
}

#[test]
fn swift_framework_import_is_external() {
    let known = paths(&["App.swift"]);
    let result = resolve_one("swift", "App.swift", "import Foundation", &known);
    assert_eq!(result.kind.as_sql_text(), "External");
}

#[test]
fn csharp_using_directive_resolves_to_the_real_file() {
    let known = paths(&["Core/Helper.cs", "Program.cs"]);
    let result = resolve_one("csharp", "Program.cs", "using Core.Helper;", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("Core/Helper.cs"));
}

#[test]
fn csharp_using_system_is_unresolved_not_faked() {
    let known = paths(&["Program.cs"]);
    let result = resolve_one("csharp", "Program.cs", "using System.IO;", &known);
    assert_eq!(result.kind.as_sql_text(), "Unresolved");
}

#[test]
fn dart_relative_import_resolves_to_the_real_file() {
    let known = paths(&["util.dart", "app.dart"]);
    let result = resolve_one("dart", "app.dart", "import '../util.dart';", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("util.dart"));
}

#[test]
fn dart_package_import_is_external() {
    let known = paths(&["app.dart"]);
    let result = resolve_one("dart", "app.dart", "import 'package:foo/bar.dart';", &known);
    assert_eq!(result.kind.as_sql_text(), "External");
}

#[test]
fn c_quoted_include_resolves_to_the_real_header() {
    let known = paths(&["local.h", "main.c"]);
    let result = resolve_one("c", "main.c", "#include \"local.h\"", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("local.h"));
}

#[test]
fn c_system_include_is_unresolved_not_faked() {
    let known = paths(&["main.c"]);
    let result = resolve_one("c", "main.c", "#include <stdio.h>", &known);
    assert_eq!(result.kind.as_sql_text(), "Unresolved");
}

#[test]
fn perl_use_is_external_for_bare_module() {
    let known = paths(&["app.pl"]);
    let result = resolve_one("perl", "app.pl", "use strict;", &known);
    assert_eq!(result.kind.as_sql_text(), "External");
}

#[test]
fn php_namespace_use_resolves_to_the_real_file() {
    let known = paths(&["Foo/Bar/Baz.php", "app.php"]);
    let result = resolve_one("php", "app.php", "use Foo\\Bar\\Baz;", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("Foo/Bar/Baz.php"));
}

#[test]
fn perl_use_resolves_to_real_module_file() {
    let known = paths(&["My/Module.pm", "app.pl"]);
    let result = resolve_one("perl", "app.pl", "use My::Module;", &known);
    assert_eq!(result.kind.as_sql_text(), "Resolved");
    assert_eq!(result.to_relative_path.as_deref(), Some("My/Module.pm"));
}
