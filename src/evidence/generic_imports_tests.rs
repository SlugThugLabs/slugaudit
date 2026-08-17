use super::*;
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin};

fn import_sources(items: &[EvidenceItem]) -> Vec<&str> {
    items
        .iter()
        .filter(|item| item.kind == EvidenceKind::Import)
        .filter_map(|item| item.payload.get("source").and_then(|v| v.as_str()))
        .collect()
}

#[test]
fn kotlin_import_header_is_captured() {
    let items = extract_generic_imports("kotlin", "import kotlin.math.max\nimport util.Helper\n");
    let sources = import_sources(&items);
    assert_eq!(
        sources,
        vec!["import kotlin.math.max", "import util.Helper"]
    );
    assert!(
        items
            .iter()
            .all(|i| i.origin == EvidenceOrigin::GenericWalker)
    );
}

#[test]
fn swift_import_declaration_is_captured() {
    let items = extract_generic_imports("swift", "import Foundation\nimport MyModule\n");
    assert_eq!(
        import_sources(&items),
        vec!["import Foundation", "import MyModule"]
    );
}

#[test]
fn csharp_using_directive_is_captured() {
    let items = extract_generic_imports("csharp", "using System.IO;\nusing MyApp.Core;\n");
    assert_eq!(
        import_sources(&items),
        vec!["using System.IO;", "using MyApp.Core;"]
    );
}

#[test]
fn dart_import_specification_is_captured() {
    let items = extract_generic_imports(
        "dart",
        "import 'dart:async';\nimport '../util.dart';\nimport 'package:foo/bar.dart';\n",
    );
    let sources = import_sources(&items);
    assert!(
        sources.contains(&"import '../util.dart';"),
        "expected the relative dart import among {sources:?}"
    );
}

#[test]
fn julia_import_and_using_are_captured() {
    let items = extract_generic_imports("julia", "using LinearAlgebra\nimport Pkg\n");
    assert_eq!(
        import_sources(&items),
        vec!["using LinearAlgebra", "import Pkg"]
    );
}

#[test]
fn c_preproc_include_is_captured() {
    let items = extract_generic_imports("c", "#include <stdio.h>\n#include \"local.h\"\n");
    assert_eq!(
        import_sources(&items),
        vec!["#include <stdio.h>", "#include \"local.h\""]
    );
}

#[test]
fn cpp_preproc_include_is_captured() {
    let items = extract_generic_imports("cpp", "#include <vector>\n#include \"util.hpp\"\n");
    assert_eq!(
        import_sources(&items),
        vec!["#include <vector>", "#include \"util.hpp\""]
    );
}

#[test]
fn haskell_import_is_captured() {
    let items = extract_generic_imports(
        "haskell",
        "import Data.List\nimport qualified Data.Map as M\n",
    );
    assert_eq!(
        import_sources(&items),
        vec!["import Data.List", "import qualified Data.Map as M"]
    );
}

#[test]
fn perl_use_statement_is_captured() {
    let items = extract_generic_imports("perl", "use strict;\nuse lib 'lib';\n");
    assert_eq!(
        import_sources(&items),
        vec!["use strict;", "use lib 'lib';"]
    );
}

#[test]
fn php_namespace_use_is_captured() {
    let items = extract_generic_imports("php", "<?php\nuse Foo\\Bar\\Baz;\nuse MyApp\\Core;\n");
    let sources = import_sources(&items);
    assert!(
        sources.contains(&"use Foo\\Bar\\Baz;"),
        "expected the php namespace use among {sources:?}"
    );
}

#[test]
fn unknown_language_returns_nothing_without_error() {
    assert!(extract_generic_imports("not-a-real-language", "import x\n").is_empty());
}

/// C7: the exact import-kind list must not fire on ordinary non-import
/// constructs in any matrix language. A source with no import statement
/// must produce zero Import items from the generic walker — this guards
/// against a future kind being added that collides with, say, a function
/// or expression node in an unforeseen grammar.
#[test]
fn c7_no_imports_source_produces_no_import_items_in_any_matrix_language() {
    for (language, source) in [
        ("rust", "fn f() { let x = 1; x }\n"),
        ("python", "def f():\n    return 1\n"),
        ("javascript", "function f() { return 1; }\n"),
        ("typescript", "function f(): number { return 1; }\n"),
        ("go", "func f() int { return 1 }\n"),
        ("c", "int f(void) { return 1; }\n"),
        ("cpp", "int f() { return 1; }\n"),
        ("swift", "func f() -> Int { return 1 }\n"),
        ("kotlin", "fun f(): Int = 1\n"),
        ("csharp", "int F() { return 1; }\n"),
        ("dart", "int f() => 1;\n"),
        ("julia", "f() = 1\n"),
        ("php", "<?php function f() { return 1; }\n"),
        ("perl", "sub f { return 1 }\n"),
        ("ocaml", "let f () = 1\n"),
        ("elixir", "defmodule M do\n  def f do\n    1\n  end\nend\n"),
        ("ruby", "def f\n  1\nend\n"),
        ("java", "int f() { return 1; }\n"),
    ] {
        let items = extract_generic_imports(language, source);
        assert_eq!(
            import_sources(&items),
            Vec::<&str>::new(),
            "{language}: a function with no imports must produce no Import items"
        );
    }
}

/// C7: the matrix languages' real import statements must still be
/// captured — exact matching must not have dropped coverage for a
/// language whose kind was previously matched by substring.
#[test]
fn c7_matrix_languages_still_capture_their_real_imports() {
    for (language, source, expected) in [
        ("swift", "import Foundation\n", vec!["import Foundation"]),
        (
            "kotlin",
            "import kotlin.math.max\n",
            vec!["import kotlin.math.max"],
        ),
        ("csharp", "using System.IO;\n", vec!["using System.IO;"]),
        (
            "dart",
            "import 'dart:async';\n",
            vec!["import 'dart:async';"],
        ),
        (
            "julia",
            "using LinearAlgebra\n",
            vec!["using LinearAlgebra"],
        ),
        ("c", "#include <stdio.h>\n", vec!["#include <stdio.h>"]),
        ("cpp", "#include <vector>\n", vec!["#include <vector>"]),
        ("haskell", "import Data.List\n", vec!["import Data.List"]),
        ("perl", "use strict;\n", vec!["use strict;"]),
        (
            "php",
            "<?php\nuse Foo\\Bar\\Baz;\n",
            vec!["use Foo\\Bar\\Baz;"],
        ),
    ] {
        let items = extract_generic_imports(language, source);
        assert_eq!(
            import_sources(&items),
            expected,
            "{language}: the real import statement must still be captured"
        );
    }
}

#[test]
fn spans_are_present_and_payloads_are_pack_shaped() {
    let items = extract_generic_imports("swift", "import Foundation\n");
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0].span, SpanAvailability::Present(_)));
    assert_eq!(
        items[0].payload["is_wildcard"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(items[0].payload["items"], serde_json::Value::Array(vec![]));
}
