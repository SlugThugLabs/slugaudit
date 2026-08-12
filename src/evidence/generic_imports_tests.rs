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
