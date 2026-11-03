use crate::graph::resolver::resolve_one;
use crate::graph::test_support::paths;
use slugaudit_test_check::{TestResult, ensure_eq};

#[test]
fn python_dot_import_resolves_to_package_init() -> TestResult {
    let known = paths(&["pkg/__init__.py", "pkg/a.py"]);
    let result = resolve_one("python", "pkg/a.py", ".", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("pkg/__init__.py"),
        "result.to_relative_path.as_deref() == Some(\"pkg/__init__.py\")",
    )?;
    Ok(())
}

#[test]
fn python_single_dot_sibling_module() -> TestResult {
    let known = paths(&["pkg/a.py", "pkg/bar.py"]);
    let result = resolve_one("python", "pkg/a.py", ".bar", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("pkg/bar.py"),
        "result.to_relative_path.as_deref() == Some(\"pkg/bar.py\")",
    )?;
    Ok(())
}

#[test]
fn python_double_dot_goes_up_one_package() -> TestResult {
    let known = paths(&["pkg/sub/a.py", "pkg/mod.py"]);
    let result = resolve_one("python", "pkg/sub/a.py", "..mod", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("pkg/mod.py"),
        "result.to_relative_path.as_deref() == Some(\"pkg/mod.py\")",
    )?;
    Ok(())
}

#[test]
fn python_dotted_path_prefers_package_form() -> TestResult {
    let known = paths(&["pkg/a.py", "pkg/sub/__init__.py"]);
    let result = resolve_one("python", "pkg/a.py", ".sub", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("pkg/sub/__init__.py"),
        "result.to_relative_path.as_deref() == Some(\"pkg/sub/__init__.py\")",
    )?;
    Ok(())
}

#[test]
fn python_absolute_import_is_external() -> TestResult {
    let known = paths(&["pkg/a.py"]);
    let result = resolve_one("python", "pkg/a.py", "os", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"External",
        "result.kind.as_sql_text() == \"External\"",
    )?;
    ensure_eq(
        &result.to_relative_path,
        &None,
        "result.to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn python_relative_import_with_no_matching_file_is_unresolved() -> TestResult {
    let known = paths(&["pkg/a.py"]);
    let result = resolve_one("python", "pkg/a.py", ".missing", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    Ok(())
}

#[test]
fn js_relative_import_resolves_with_guessed_extension() -> TestResult {
    let known = paths(&["src/a.ts", "src/utils.ts"]);
    let result = resolve_one("javascript", "src/a.ts", "./utils", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/utils.ts"),
        "result.to_relative_path.as_deref() == Some(\"src/utils.ts\")",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("High"),
        "result.confidence == Some(\"High\")",
    )?;
    Ok(())
}

#[test]
fn js_relative_import_resolves_to_directory_index() -> TestResult {
    let known = paths(&["src/a.ts", "src/lib/index.ts"]);
    let result = resolve_one("javascript", "src/pages/a.ts", "../lib", &known);
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("src/lib/index.ts"),
        "result.to_relative_path.as_deref() == Some(\"src/lib/index.ts\")",
    )?;
    Ok(())
}

#[test]
fn js_ambiguous_extension_match_is_low_confidence() -> TestResult {
    let known = paths(&["src/a.ts", "src/utils.ts", "src/utils.js"]);
    let result = resolve_one("javascript", "src/a.ts", "./utils", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.confidence,
        &Some("Low"),
        "result.confidence == Some(\"Low\")",
    )?;
    Ok(())
}

#[test]
fn js_bare_package_name_is_external() -> TestResult {
    let known = paths(&["src/a.ts"]);
    let result = resolve_one("javascript", "src/a.ts", "react", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"External",
        "result.kind.as_sql_text() == \"External\"",
    )?;
    Ok(())
}

#[test]
fn an_unsupported_language_is_always_unresolved() -> TestResult {
    let known = paths(&["a.go"]);
    let result = resolve_one("go", "a.go", "./local", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    Ok(())
}

#[test]
fn kotlin_module_import_resolves_to_the_real_file() -> TestResult {
    let known = paths(&["util/Helper.kt", "main.kt"]);
    let result = resolve_one("kotlin", "main.kt", "import util.Helper", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("util/Helper.kt"),
        "result.to_relative_path.as_deref() == Some(\"util/Helper.kt\")",
    )?;
    Ok(())
}

#[test]
fn kotlin_stdlib_import_is_not_faked_resolved() -> TestResult {
    let known = paths(&["main.kt"]);
    let result = resolve_one("kotlin", "main.kt", "import kotlin.math.max", &known);
    // A dotted stdlib module can't be told apart from a project module
    // without a file match — the same honest Unresolved verdict python
    // gives `import os.path`. The point: never faked as Resolved.
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path,
        &None,
        "result.to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn swift_framework_import_is_external() -> TestResult {
    let known = paths(&["App.swift"]);
    let result = resolve_one("swift", "App.swift", "import Foundation", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"External",
        "result.kind.as_sql_text() == \"External\"",
    )?;
    Ok(())
}

#[test]
fn csharp_using_directive_resolves_to_the_real_file() -> TestResult {
    let known = paths(&["Core/Helper.cs", "Program.cs"]);
    let result = resolve_one("csharp", "Program.cs", "using Core.Helper;", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("Core/Helper.cs"),
        "result.to_relative_path.as_deref() == Some(\"Core/Helper.cs\")",
    )?;
    Ok(())
}

#[test]
fn csharp_using_system_is_unresolved_not_faked() -> TestResult {
    let known = paths(&["Program.cs"]);
    let result = resolve_one("csharp", "Program.cs", "using System.IO;", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    Ok(())
}

#[test]
fn dart_relative_import_resolves_to_the_real_file() -> TestResult {
    let known = paths(&["util.dart", "lib/app.dart"]);
    let result = resolve_one("dart", "lib/app.dart", "import '../util.dart';", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("util.dart"),
        "result.to_relative_path.as_deref() == Some(\"util.dart\")",
    )?;
    Ok(())
}

#[test]
fn dart_package_import_is_external() -> TestResult {
    let known = paths(&["app.dart"]);
    let result = resolve_one("dart", "app.dart", "import 'package:foo/bar.dart';", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"External",
        "result.kind.as_sql_text() == \"External\"",
    )?;
    Ok(())
}

#[test]
fn c_quoted_include_resolves_to_the_real_header() -> TestResult {
    let known = paths(&["local.h", "main.c"]);
    let result = resolve_one("c", "main.c", "#include \"local.h\"", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("local.h"),
        "result.to_relative_path.as_deref() == Some(\"local.h\")",
    )?;
    Ok(())
}

#[test]
fn c_system_include_is_unresolved_not_faked() -> TestResult {
    let known = paths(&["main.c"]);
    let result = resolve_one("c", "main.c", "#include <stdio.h>", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    Ok(())
}

#[test]
fn perl_use_is_external_for_bare_module() -> TestResult {
    let known = paths(&["app.pl"]);
    let result = resolve_one("perl", "app.pl", "use strict;", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"External",
        "result.kind.as_sql_text() == \"External\"",
    )?;
    Ok(())
}

#[test]
fn php_namespace_use_resolves_to_the_real_file() -> TestResult {
    let known = paths(&["Foo/Bar/Baz.php", "app.php"]);
    let result = resolve_one("php", "app.php", "use Foo\\Bar\\Baz;", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("Foo/Bar/Baz.php"),
        "result.to_relative_path.as_deref() == Some(\"Foo/Bar/Baz.php\")",
    )?;
    Ok(())
}

#[test]
fn perl_use_resolves_to_real_module_file() -> TestResult {
    let known = paths(&["My/Module.pm", "app.pl"]);
    let result = resolve_one("perl", "app.pl", "use My::Module;", &known);
    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("My/Module.pm"),
        "result.to_relative_path.as_deref() == Some(\"My/Module.pm\")",
    )?;
    Ok(())
}

#[test]
fn a_relative_import_that_escapes_the_project_root_is_unresolved() -> TestResult {
    let known = paths(&["x.ts", "a.ts"]);

    let result = resolve_one("typescript", "a.ts", "../x", &known);

    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path,
        &None,
        "result.to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn a_relative_import_that_climbs_past_the_root_from_a_subdirectory_is_unresolved() -> TestResult {
    let known = paths(&["x.ts", "src/a.ts"]);

    let result = resolve_one("typescript", "src/a.ts", "../../x", &known);

    ensure_eq(
        &result.to_relative_path,
        &None,
        "result.to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn a_relative_import_that_stays_inside_the_root_still_resolves() -> TestResult {
    let known = paths(&["x.ts", "src/a.ts"]);

    let result = resolve_one("typescript", "src/a.ts", "../x", &known);

    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("x.ts"),
        "result.to_relative_path.as_deref() == Some(\"x.ts\")",
    )?;
    Ok(())
}

#[test]
fn python_double_dot_from_a_root_level_file_is_unresolved() -> TestResult {
    let known = paths(&["a.py", "bar.py"]);

    let result = resolve_one("python", "a.py", "..bar", &known);

    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path,
        &None,
        "result.to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn python_extra_dots_that_climb_past_the_root_from_a_subdirectory_are_unresolved() -> TestResult {
    let known = paths(&["pkg/a.py", "bar.py"]);

    let result = resolve_one("python", "pkg/a.py", "...bar", &known);

    ensure_eq(
        &result.kind.as_sql_text(),
        &"Unresolved",
        "result.kind.as_sql_text() == \"Unresolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path,
        &None,
        "result.to_relative_path == None",
    )?;
    Ok(())
}

#[test]
fn python_double_dot_from_a_first_level_package_reaches_the_root_module() -> TestResult {
    let known = paths(&["pkg/a.py", "bar.py"]);

    let result = resolve_one("python", "pkg/a.py", "..bar", &known);

    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("bar.py"),
        "result.to_relative_path.as_deref() == Some(\"bar.py\")",
    )?;
    Ok(())
}

#[test]
fn python_dot_import_at_the_project_root_resolves_the_root_package_init() -> TestResult {
    let known = paths(&["__init__.py", "a.py"]);

    let result = resolve_one("python", "a.py", ".", &known);

    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("__init__.py"),
        "result.to_relative_path.as_deref() == Some(\"__init__.py\")",
    )?;
    Ok(())
}

#[test]
fn python_double_dot_from_a_first_level_package_resolves_the_root_package_init() -> TestResult {
    let known = paths(&["__init__.py", "pkg/a.py"]);

    let result = resolve_one("python", "pkg/a.py", "..", &known);

    ensure_eq(
        &result.kind.as_sql_text(),
        &"Resolved",
        "result.kind.as_sql_text() == \"Resolved\"",
    )?;
    ensure_eq(
        &result.to_relative_path.as_deref(),
        &Some("__init__.py"),
        "result.to_relative_path.as_deref() == Some(\"__init__.py\")",
    )?;
    Ok(())
}
