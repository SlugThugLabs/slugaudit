"""Tests for all 8 language extractors — signature and import extraction."""

import os
import tempfile
import unittest

from languages import (
    RustExtractor,
    PythonExtractor,
    TypeScriptExtractor,
    GoExtractor,
    JavaExtractor,
    CExtractor,
    CppExtractor,
    RubyExtractor,
)


class TestRustExtractor(unittest.TestCase):
    """Rust extractor: functions, structs, enums, traits, impls, imports."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = RustExtractor(self.tmpdir)

    def test_extracts_fn(self) -> None:
        source = b"pub fn hello(name: &str) -> String {\n    format!(\"Hello {}\", name)\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "fn")
        self.assertEqual(sigs[0]["name"], "hello")
        self.assertEqual(sigs[0]["visibility"], "pub")

    def test_extracts_private_fn(self) -> None:
        source = b"fn internal() {}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["visibility"], "")

    def test_extracts_struct(self) -> None:
        source = b"pub struct Point {\n    x: i32,\n    y: i32,\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "struct")
        self.assertEqual(sigs[0]["name"], "Point")
        self.assertEqual(sigs[0]["visibility"], "pub")

    def test_extracts_enum(self) -> None:
        source = b"enum Color {\n    Red,\n    Green,\n    Blue,\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "enum")
        self.assertEqual(sigs[0]["name"], "Color")

    def test_extracts_trait(self) -> None:
        source = b"pub trait Drawable {\n    fn draw(&self);\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        types = {s["type"] for s in sigs}
        self.assertIn("trait", types)
        trait_sigs = [s for s in sigs if s["type"] == "trait"]
        self.assertEqual(trait_sigs[0]["name"], "Drawable")

    def test_extracts_impl(self) -> None:
        source = b"impl Point {\n    fn new(x: i32, y: i32) -> Self { Self { x, y } }\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertTrue(any(s["type"] == "impl" for s in sigs))

    def test_extracts_use_import(self) -> None:
        source = b"use std::collections::HashMap;\n"
        imps = self.ext.extract_imports("/tmp/test.rs", source)
        self.assertEqual(len(imps), 1)
        self.assertIn("HashMap", imps[0]["import_text"])

    def test_extracts_multiple_items(self) -> None:
        source = b"pub fn foo() {}\nfn bar() {}\npub struct Baz {}\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertEqual(len(sigs), 3)

    def test_extracts_type_alias(self) -> None:
        source = b"pub type Result<T> = std::result::Result<T, Error>;\n"
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "type_alias")
        self.assertEqual(sigs[0]["name"], "Result")

    def test_extracts_let_bindings_including_tuple_patterns(self) -> None:
        source = (
            b"fn f() {\n"
            b"    let x = 1;\n"
            b"    let mut y: i32 = 2;\n"
            b"    let (a, b) = (1, 2);\n"
            b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.rs", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"x", "y", "a", "b"})

    def test_deeply_nested_source_does_not_raise_recursion_error(self) -> None:
        # Well past Python's default recursion limit (~1000). A recursive
        # tree walker would raise RecursionError on this; the iterative
        # walkers in languages/base.py and RustExtractor._walk_risk must not.
        depth = 4000
        source = (
            b"fn deep() -> i32 {\n"
            + b"if true {\n" * depth
            + b"1.unwrap()"
            + b"} else { 0 }\n" * depth
            + b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/deep.rs", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["name"], "deep")

        risks = self.ext.extract_risk_patterns("/tmp/deep.rs", source)
        risk_types = {r["pattern_type"] for r in risks}
        self.assertIn("unwrap", risk_types)


class TestRustImportResolution(unittest.TestCase):
    """Brace-group expansion and cross-crate / `crate::` resolution.

    Regression coverage for the four rust.py bugs: cross-crate imports being
    abandoned, `crate::` resolving against the wrong root, brace groups never
    being expanded, and unknown crates being misclassified as internal.
    """

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()

    def _write(self, relpath: str, content: str) -> None:
        full = os.path.join(self.tmpdir, relpath)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as fh:
            fh.write(content)

    def _make_workspace(self) -> RustExtractor:
        """A tiny multi-crate workspace mirroring slugid's shape.

        No [workspace] members list is relied upon — each crate is
        discovered purely by globbing for Cargo.toml files, as slugid itself
        declares its members as root path-dependencies rather than a
        [workspace] table.
        """
        self._write("Cargo.toml", '[package]\nname = "slugid-guardrails"\nversion = "0.1.0"\n')
        self._write("contracts/Cargo.toml", '[package]\nname = "contracts"\nversion = "0.1.0"\n')
        self._write("application/Cargo.toml", '[package]\nname = "application"\nversion = "0.1.0"\n')
        self._write(
            "infrastructure/Cargo.toml",
            '[package]\nname = "slugid-infrastructure"\nversion = "0.1.0"\n',
        )
        return RustExtractor(self.tmpdir)

    def test_nested_brace_expansion(self) -> None:
        ext = self._make_workspace()
        source = b"use a::{b::{c, d}, e};\n"
        imps = ext.extract_imports("/tmp/test.rs", source)
        texts = {i["import_text"] for i in imps}
        self.assertEqual(texts, {"use a::b::c;", "use a::b::d;", "use a::e;"})

    def test_self_in_brace_group(self) -> None:
        ext = self._make_workspace()
        source = b"use a::{self, b};\n"
        imps = ext.extract_imports("/tmp/test.rs", source)
        texts = {i["import_text"] for i in imps}
        self.assertEqual(texts, {"use a;", "use a::b;"})

    def test_glob_import_resolves_to_module(self) -> None:
        ext = self._make_workspace()
        source = b"use a::b::*;\n"
        imps = ext.extract_imports("/tmp/test.rs", source)
        self.assertEqual(len(imps), 1)
        self.assertEqual(imps[0]["import_text"], "use a::b;")

    def test_crate_map_normalizes_dash_to_underscore(self) -> None:
        ext = self._make_workspace()
        self.assertIn("slugid_infrastructure", ext.crate_map)
        self.assertEqual(ext.crate_map["slugid_infrastructure"], "infrastructure/src")
        self.assertEqual(ext.crate_map["contracts"], "contracts/src")
        self.assertEqual(ext.crate_map["slugid_guardrails"], "src")

    def test_cross_crate_import_resolves(self) -> None:
        ext = self._make_workspace()
        path_to_id = {"contracts/src/ports.rs": "id-ports"}
        resolved = ext.resolve_import(
            "use contracts::ports::ApplicationService;",
            "client/src/app.rs",
            path_to_id,
        )
        self.assertEqual(resolved, "contracts/src/ports.rs")

    def test_cross_crate_dash_name_resolves(self) -> None:
        ext = self._make_workspace()
        path_to_id = {"infrastructure/src/config.rs": "id-config"}
        resolved = ext.resolve_import(
            "use slugid_infrastructure::config;",
            "domain/src/lib.rs",
            path_to_id,
        )
        self.assertEqual(resolved, "infrastructure/src/config.rs")

    def test_crate_prefix_resolves_against_owning_crate_not_project_root(self) -> None:
        ext = self._make_workspace()
        path_to_id = {"application/src/services/registry.rs": "id-registry"}
        # crate:: inside a non-root workspace member (application/src/services/mod.rs)
        # must resolve against application/src, not <project_root>/src.
        resolved = ext.resolve_import(
            "use crate::services::registry::Registry;",
            "application/src/services/mod.rs",
            path_to_id,
        )
        self.assertEqual(resolved, "application/src/services/registry.rs")

    def test_external_crate_not_resolved(self) -> None:
        ext = self._make_workspace()
        resolved = ext.resolve_import("use egui::Ui;", "client/src/app.rs", {})
        self.assertIsNone(resolved)

    def test_classify_unknown_crate_as_external(self) -> None:
        ext = self._make_workspace()
        source = (
            b"use egui::Ui;\n"
            b"use serde::Deserialize;\n"
            b"use contracts::ports::ApplicationService;\n"
        )
        imps = ext.extract_imports("/tmp/test.rs", source)
        by_text = {i["import_text"]: i["import_type"] for i in imps}
        self.assertEqual(by_text["use egui::Ui;"], "external")
        self.assertEqual(by_text["use serde::Deserialize;"], "external")
        self.assertEqual(by_text["use contracts::ports::ApplicationService;"], "internal")

    def test_resolution_never_points_outside_indexed_files(self) -> None:
        """Never resolves to a path absent from path_to_id (the indexed file map)."""
        ext = self._make_workspace()
        path_to_id = {"contracts/src/ports.rs": "id-ports"}
        resolved = ext.resolve_import(
            "use contracts::ports::SomethingNotIndexed;",
            "client/src/app.rs",
            path_to_id,
        )
        # The leaf candidates don't exist in the index; only the
        # parent-module fallback does.
        self.assertEqual(resolved, "contracts/src/ports.rs")

        no_match = ext.resolve_import(
            "use contracts::nonexistent::Thing;",
            "client/src/app.rs",
            path_to_id,
        )
        self.assertIsNone(no_match)


class TestPythonExtractor(unittest.TestCase):
    """Python extractor: functions, classes, imports."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = PythonExtractor(self.tmpdir)

    def test_extracts_fn(self) -> None:
        source = b"def hello(name: str) -> str:\n    return f'Hello {name}'\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "fn")
        self.assertEqual(sigs[0]["name"], "hello")

    def test_extracts_async_fn(self) -> None:
        source = b"async def fetch(url: str) -> bytes:\n    return b''\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        self.assertEqual(len(sigs), 1)
        self.assertTrue(sigs[0]["is_async"])

    def test_extracts_class(self) -> None:
        source = b"class MyClass:\n    pass\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "class")
        self.assertEqual(sigs[0]["name"], "MyClass")

    def test_extracts_class_with_bases(self) -> None:
        source = b"class Derived(Base, Mixin):\n    pass\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "class")
        self.assertIn("Base", sigs[0]["signature"])
        self.assertIn("Mixin", sigs[0]["signature"])

    def test_extracts_import(self) -> None:
        source = b"import os\nimport sys\n"
        imps = self.ext.extract_imports("/tmp/test.py", source)
        self.assertEqual(len(imps), 2)
        self.assertIn("os", imps[0]["import_text"])

    def test_extracts_from_import(self) -> None:
        source = b"from collections import OrderedDict\n"
        imps = self.ext.extract_imports("/tmp/test.py", source)
        self.assertEqual(len(imps), 1)
        self.assertIn("OrderedDict", imps[0]["import_text"])

    def test_extracts_decorated_fn_exactly_once(self) -> None:
        # Regression test: decorated_definition used to have a special case
        # that extracted its wrapped function_definition child directly,
        # *and* the walker extracted that same child again naturally,
        # recording every decorated function twice. Fixed by removing the
        # special case entirely — the walker's plain FN_DEF dispatch was
        # always sufficient (this is exactly how decorated classes already
        # worked, since CLASS_DEF was never special-cased).
        source = b"@decorator\ndef wrapped():\n    pass\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        names = [s["name"] for s in sigs]
        self.assertEqual(names.count("wrapped"), 1)

    def test_extracts_multiply_decorated_async_fn_exactly_once(self) -> None:
        source = b"@staticmethod\n@cache\nasync def fetch(x):\n    pass\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        names = [s["name"] for s in sigs]
        self.assertEqual(names.count("fetch"), 1)
        self.assertTrue(sigs[0]["is_async"])

    def test_extracts_module_and_local_variables_including_self_attrs(self) -> None:
        source = (
            b"TOP = 1\n"
            b"def f():\n"
            b"    x = 1\n"
            b"    y: int = 2\n"
            b"    a, b = 1, 2\n"
            b"class C:\n"
            b"    def __init__(self):\n"
            b"        self.z = 1\n"
            b"        self.p, self.q = 1, 2\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"TOP", "x", "y", "a", "b", "z", "p", "q"})

    def test_type_annotation_identifier_is_not_mistaken_for_a_second_name(self) -> None:
        # `y: int = 2` puts an `identifier` node ("int") inside the type
        # annotation too — only the actual target ("y") must be extracted.
        source = b"def f():\n    y: int = 2\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        variables = [s["name"] for s in sigs if s["type"] == "variable"]
        self.assertEqual(variables, ["y"])


class TestTypeScriptExtractor(unittest.TestCase):
    """TypeScript extractor: functions, classes, interfaces, enums, imports."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = TypeScriptExtractor(self.tmpdir)

    def test_extracts_fn(self) -> None:
        source = b"function hello(name: string): string {\n  return `Hello ${name}`;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "fn")
        self.assertEqual(sigs[0]["name"], "hello")

    def test_extracts_exported_fn(self) -> None:
        source = b"export function add(a: number, b: number): number {\n  return a + b;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["visibility"], "export")

    def test_extracts_class(self) -> None:
        source = b"class Animal {\n  constructor(name: string) {}\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "class")
        self.assertEqual(sigs[0]["name"], "Animal")

    def test_extracts_top_level_and_local_variables(self) -> None:
        source = (
            b"const TOP = 1;\n"
            b"function f() {\n"
            b"    let x = 1;\n"
            b"    var y = 2;\n"
            b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"TOP", "x", "y"})

    def test_extracts_interface(self) -> None:
        source = b"interface User {\n  name: string;\n  age: number;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "interface")
        self.assertEqual(sigs[0]["name"], "User")

    def test_extracts_enum(self) -> None:
        source = b"enum Status {\n  Active,\n  Inactive,\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "enum")
        self.assertEqual(sigs[0]["name"], "Status")

    def test_extracts_type_alias(self) -> None:
        source = b"type Point = { x: number; y: number };\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "type_alias")
        self.assertEqual(sigs[0]["name"], "Point")

    def test_extracts_import(self) -> None:
        source = b"import { Component } from 'react';\n"
        imps = self.ext.extract_imports("/tmp/test.ts", source)
        self.assertEqual(len(imps), 1)
        self.assertIn("Component", imps[0]["import_text"])

    def test_extracts_var_variable(self) -> None:
        source = b"var NAME = \"slugaudit\";\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "variable")
        self.assertEqual(sigs[0]["name"], "NAME")

    def test_extracts_const_variable(self) -> None:
        source = b"const MAX_SIZE = 100;\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "variable")
        self.assertEqual(sigs[0]["name"], "MAX_SIZE")

    def test_const_bound_async_arrow_function_is_extracted_as_a_function(self) -> None:
        # Regression test: `const f = async (x) => ...` used to be flattened
        # to a generic "variable" signature with no is_async/param info at
        # all — a real gap given this is the dominant function style in a
        # lot of modern JS/TS over `function` declarations.
        source = b"const fetchData = async (url) => {\n  return await fetch(url);\n};\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "fn")
        self.assertEqual(sigs[0]["name"], "fetchData")
        self.assertTrue(sigs[0]["is_async"])

    def test_const_bound_concise_arrow_function_is_extracted_as_a_function(self) -> None:
        source = b"const add = (a, b) => a + b;\n"
        sigs = self.ext.extract_signatures("/tmp/test.ts", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "fn")
        self.assertFalse(sigs[0]["is_async"])

    def test_deeply_nested_source_does_not_raise_recursion_error(self) -> None:
        # TypeScriptExtractor has its own _walk_tree override (export handling
        # doesn't fit the shared base-class dispatch); it must be iterative
        # too, not just the shared base.py walkers.
        depth = 4000
        source = (
            b"export function deep(): number {\n"
            + b"if (true) {\n" * depth
            + b"return 1;\n"
            + b"}\n" * depth
            + b"return 0;\n}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/deep.ts", source)
        names = [s["name"] for s in sigs]
        self.assertIn("deep", names)


class TestGoExtractor(unittest.TestCase):
    """Go extractor: functions, methods, structs, interfaces, imports."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = GoExtractor(self.tmpdir)

    def test_extracts_fn(self) -> None:
        source = b"func Hello(name string) string {\n    return \"Hello \" + name\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.go", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "function")
        self.assertEqual(sigs[0]["name"], "Hello")

    def test_exported_visibility(self) -> None:
        source = b"func Hello() {}\nfunc private() {}\n"
        sigs = self.ext.extract_signatures("/tmp/test.go", source)
        self.assertEqual(len(sigs), 2)
        hello = next(s for s in sigs if s["name"] == "Hello")
        priv = next(s for s in sigs if s["name"] == "private")
        self.assertEqual(hello["visibility"], "exported")
        self.assertEqual(priv["visibility"], "")

    def test_extracts_struct(self) -> None:
        source = b"type Point struct {\n    X int\n    Y int\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.go", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "struct")
        self.assertEqual(sigs[0]["name"], "Point")

    def test_extracts_interface(self) -> None:
        source = b"type Stringer interface {\n    String() string\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.go", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "interface")
        self.assertEqual(sigs[0]["name"], "Stringer")

    def test_deeply_nested_source_does_not_raise_recursion_error(self) -> None:
        # Go uses the shared base.py _walk_tree/_walk_imports with no
        # language-specific override; this proves the shared iterative
        # walker (not just Rust's own _walk_risk) survives deep nesting.
        depth = 4000
        source = (
            b"func Deep() int {\n"
            + b"if true {\n" * depth
            + b"return 1\n"
            + b"}\n" * depth
            + b"return 0\n}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/deep.go", source)
        names = [s["name"] for s in sigs]
        self.assertIn("Deep", names)

    def test_extracts_import(self) -> None:
        source = b'import \"fmt\"\nimport \"os\"\n'
        imps = self.ext.extract_imports("/tmp/test.go", source)
        self.assertGreaterEqual(len(imps), 1)
        self.assertIn("fmt", imps[0]["import_text"])

    def test_extracts_grouped_import(self) -> None:
        source = b'import (\n\t\"fmt\"\n\t\"os\"\n)\n'
        imps = self.ext.extract_imports("/tmp/test.go", source)
        self.assertGreaterEqual(len(imps), 2)

    def test_extracts_var_const_and_short_var_declarations(self) -> None:
        source = (
            b"package main\n"
            b"var Top = 1\n"
            b"const C = 2\n"
            b"func f() {\n"
            b"    var x int = 1\n"
            b"    y := 2\n"
            b"    a, b := 1, 2\n"
            b"    _, err := f2()\n"
            b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.go", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        # The Go blank identifier `_` is never a real variable and must be excluded.
        self.assertEqual(variables, {"Top", "C", "x", "y", "a", "b", "err"})

    def test_short_var_rhs_identifier_is_not_mistaken_for_a_name(self) -> None:
        # `y := existing` has an identifier on the RHS expression_list too —
        # only the LHS expression_list's identifier is a new declaration.
        source = b"func f() {\n    existing := 1\n    y := existing\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.go", source)
        variables = [s["name"] for s in sigs if s["type"] == "variable"]
        self.assertEqual(sorted(variables), ["existing", "y"])


class TestJavaExtractor(unittest.TestCase):
    """Java extractor: classes, interfaces, enums, methods, imports."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = JavaExtractor(self.tmpdir)

    def test_extracts_class(self) -> None:
        source = b"public class Hello {\n    public static void main(String[] args) {}\n}\n"
        sigs = self.ext.extract_signatures("/tmp/Hello.java", source)
        self.assertTrue(any(s["type"] == "class" and s["name"] == "Hello" for s in sigs))

    def test_extracts_method(self) -> None:
        source = b"public class App {\n    public String greet(String name) {\n        return \"Hi\";\n    }\n}\n"
        sigs = self.ext.extract_signatures("/tmp/App.java", source)
        methods = [s for s in sigs if s["type"] == "method"]
        self.assertTrue(any(m["name"] == "greet" for m in methods))

    def test_extracts_interface(self) -> None:
        source = b"public interface Drawable {\n    void draw();\n}\n"
        sigs = self.ext.extract_signatures("/tmp/Drawable.java", source)
        self.assertTrue(any(s["type"] == "interface" and s["name"] == "Drawable" for s in sigs))

    def test_extracts_import(self) -> None:
        source = b"import java.util.List;\nimport java.util.ArrayList;\n"
        imps = self.ext.extract_imports("/tmp/Test.java", source)
        self.assertEqual(len(imps), 2)
        self.assertIn("List", imps[0]["import_text"])

    def test_extracts_fields_and_local_variables(self) -> None:
        source = (
            b"class Foo {\n"
            b"    int field = 1;\n"
            b"    static int sField = 2;\n"
            b"    void f() {\n"
            b"        int x = 1;\n"
            b"        String s = \"a\";\n"
            b"    }\n"
            b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/Foo.java", source)
        fields = {s["name"] for s in sigs if s["type"] == "field"}
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(fields, {"field", "sField"})
        self.assertEqual(variables, {"x", "s"})

    def test_extracts_multiple_names_in_one_declaration(self) -> None:
        source = b"class Foo {\n    void f() {\n        int a = 1, b = 2;\n    }\n}\n"
        sigs = self.ext.extract_signatures("/tmp/Foo.java", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"a", "b"})


class TestCExtractor(unittest.TestCase):
    """C extractor: functions, structs, enums, typedefs, includes."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = CExtractor(self.tmpdir)

    def test_extracts_fn(self) -> None:
        source = b"int add(int a, int b) {\n    return a + b;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "function")
        self.assertEqual(sigs[0]["name"], "add")

    def test_extracts_static_fn(self) -> None:
        source = b"static void helper() {}\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["visibility"], "static")

    def test_extracts_struct(self) -> None:
        source = b"struct Point {\n    int x;\n    int y;\n};\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "struct")
        self.assertEqual(sigs[0]["name"], "Point")

    def test_extracts_typedef(self) -> None:
        source = b"typedef unsigned long size_t;\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "typedef")

    def test_extracts_include(self) -> None:
        source = b'#include <stdio.h>\n#include "myheader.h"\n'
        imps = self.ext.extract_imports("/tmp/test.c", source)
        self.assertEqual(len(imps), 2)
        self.assertIn("stdio.h", imps[0]["import_text"])

    def test_classifies_includes(self) -> None:
        source = b'#include <stdlib.h>\n#include "local.h"\n'
        imps = self.ext.extract_imports("/tmp/test.c", source)
        self.assertEqual(imps[0]["import_type"], "external")  # < >
        self.assertEqual(imps[1]["import_type"], "internal")   # " "

    def test_extracts_global_local_and_multi_declarator_variables(self) -> None:
        source = (
            b"int global_arr[3];\n"
            b"int a, b = 1;\n"
            b"void f() {\n"
            b"    int x = 1;\n"
            b"    static int s = 2;\n"
            b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"global_arr", "a", "b", "x", "s"})
        s_sig = next(s for s in sigs if s["name"] == "s")
        self.assertEqual(s_sig["visibility"], "static")

    def test_function_prototype_is_not_extracted_as_a_variable(self) -> None:
        source = b"void proto(int x);\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        self.assertEqual([s for s in sigs if s["type"] == "variable"], [])

    def test_struct_typed_variable_does_not_duplicate_or_phantom_the_type(self) -> None:
        # `struct Bar { int y; } instance;` must record the struct
        # definition exactly once (the base walker's own STRUCT_SPEC dispatch
        # handles it) plus the variable `instance` — never both from the
        # `declaration`-level handling *and* the natural walk.
        source = b"struct Bar { int y; } instance;\nstruct Baz b2;\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        struct_names = [s["name"] for s in sigs if s["type"] == "struct"]
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        # struct Baz has no body here (just referencing an existing type) —
        # it must never be reported as a second, phantom definition.
        self.assertEqual(struct_names, ["Bar"])
        self.assertEqual(variables, {"instance", "b2"})

    def test_forward_declared_struct_is_not_a_phantom_definition(self) -> None:
        source = b"struct Fwd;\nenum Color;\n"
        sigs = self.ext.extract_signatures("/tmp/test.c", source)
        self.assertEqual(sigs, [])


class TestCppExtractor(unittest.TestCase):
    """C++ extractor: functions, classes, templates, namespaces, includes."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = CppExtractor(self.tmpdir)

    def test_extracts_fn(self) -> None:
        source = b"int add(int a, int b) {\n    return a + b;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "function")
        self.assertEqual(sigs[0]["name"], "add")

    def test_extracts_class(self) -> None:
        source = b"class Rectangle {\npublic:\n    int area() const { return 0; }\n};\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        self.assertTrue(any(s["type"] == "class" and s["name"] == "Rectangle" for s in sigs))

    def test_extracts_namespace(self) -> None:
        source = b"namespace mylib {\n    int fn() { return 0; }\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        self.assertTrue(any(s["type"] == "namespace" and s["name"] == "mylib" for s in sigs))

    def test_extracts_template_fn_exactly_once(self) -> None:
        # Regression test: _extract_template used to reach into and extract
        # the wrapped function_definition directly *and* let the base
        # walker visit that same node naturally afterward, recording every
        # templated function twice ("template_fn" and "function") — the
        # same duplicate-extraction bug class documented in CLAUDE.md for
        # Python's decorated definitions. Fixed by deleting the manual
        # reach-in entirely; the plain "function" dispatch already handles
        # it once, matching the README's documented "templates are captured
        # as plain classes/functions" behavior.
        source = b"template <typename T>\nT max(T a, T b) {\n    return a > b ? a : b;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        matches = [s for s in sigs if s["name"] == "max"]
        self.assertEqual(len(matches), 1, msg=f"Expected exactly one, got: {sigs}")
        self.assertEqual(matches[0]["type"], "function")

    def test_extracts_template_struct_exactly_once(self) -> None:
        source = b"template <typename T>\nstruct Box { T value; };\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        matches = [s for s in sigs if s["name"] == "Box"]
        self.assertEqual(len(matches), 1, msg=f"Expected exactly one, got: {sigs}")
        self.assertEqual(matches[0]["type"], "struct")

    def test_extracts_include(self) -> None:
        source = b'#include <iostream>\n#include "utils.hpp"\n'
        imps = self.ext.extract_imports("/tmp/test.cpp", source)
        self.assertEqual(len(imps), 2)
        self.assertIn("iostream", imps[0]["import_text"])

    def test_extracts_global_local_and_multi_declarator_variables(self) -> None:
        source = (
            b"int global_arr[3];\n"
            b"int a, b = 1;\n"
            b"void f() {\n"
            b"    int x = 1;\n"
            b"    static int s = 2;\n"
            b"}\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"global_arr", "a", "b", "x", "s"})

    def test_function_prototype_is_not_extracted_as_a_variable(self) -> None:
        source = b"void proto(int x);\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        self.assertEqual([s for s in sigs if s["type"] == "variable"], [])

    def test_class_typed_variable_does_not_duplicate_or_phantom_the_type(self) -> None:
        source = b"class Bar { int y; } instance;\nclass Baz b2;\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        class_names = [s["name"] for s in sigs if s["type"] == "class"]
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(class_names, ["Bar"])
        self.assertEqual(variables, {"instance", "b2"})

    def test_forward_declared_class_is_not_a_phantom_definition(self) -> None:
        source = b"class Fwd;\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        self.assertEqual(sigs, [])


class TestRubyExtractor(unittest.TestCase):
    """Ruby extractor: methods, classes, modules, require calls."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = RubyExtractor(self.tmpdir)

    def test_extracts_method(self) -> None:
        source = b"def hello(name)\n  \"Hello #{name}\"\nend\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "method")
        self.assertEqual(sigs[0]["name"], "hello")

    def test_extracts_class_exactly_once(self) -> None:
        # Regression test: tree-sitter-ruby gives the class_definition node
        # and its own literal "class" keyword token the same .type string
        # ("class") — one named, one anonymous. Walking node.children
        # (rather than named_children) used to dispatch on both, recording
        # a bogus second "unnamed" class signature for the keyword token.
        source = b"class MyClass\n  def initialize\n  end\nend\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        class_sigs = [s for s in sigs if s["type"] == "class"]
        self.assertEqual(len(class_sigs), 1)
        self.assertEqual(class_sigs[0]["name"], "MyClass")

    def test_extracts_module_exactly_once(self) -> None:
        # Same bug, same fix, for "module" (see test_extracts_class_exactly_once).
        source = b"module Utilities\n  def helper\n  end\nend\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        module_sigs = [s for s in sigs if s["type"] == "module"]
        self.assertEqual(len(module_sigs), 1)
        self.assertEqual(module_sigs[0]["name"], "Utilities")

    def test_extracts_singleton_method(self) -> None:
        source = b"def self.factory_method\n  new\nend\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        self.assertEqual(len(sigs), 1)
        self.assertEqual(sigs[0]["type"], "singleton_method")

    def test_extracts_require(self) -> None:
        source = b"require 'json'\nrequire 'net/http'\n"
        imps = self.ext.extract_imports("/tmp/test.rb", source)
        self.assertEqual(len(imps), 2)
        self.assertIn("json", imps[0]["import_text"])

    def test_extracts_globals_constants_and_multiple_assignment(self) -> None:
        source = b"$g = 1\nTOP = 2\na, b = 1, 2\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        constants = {s["name"] for s in sigs if s["type"] == "constant"}
        self.assertEqual(variables, {"$g", "a", "b"})
        self.assertEqual(constants, {"TOP"})

    def test_extracts_instance_and_class_variables_including_multi_assign(self) -> None:
        source = (
            b"class Foo\n"
            b"  def initialize\n"
            b"    @iv = 1\n"
            b"    @@cv = 2\n"
            b"    @y, @z = 1, 2\n"
            b"  end\n"
            b"end\n"
        )
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        variables = {s["name"] for s in sigs if s["type"] == "variable"}
        self.assertEqual(variables, {"@iv", "@@cv", "@y", "@z"})

    def test_element_and_attribute_assignment_are_not_new_variables(self) -> None:
        # `h[:k] = 1` (element_reference) and `obj.attr = 1` (a setter-method
        # call, not a new binding) must never be reported as variables.
        source = b"h[:k] = 1\nobj.attr = 1\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        self.assertEqual(sigs, [])


class TestAllExtractors(unittest.TestCase):
    """Cross-cutting concerns: all extractors have required attributes."""

    def test_all_have_name(self) -> None:
        from languages import LANG_MAP
        for lang, cls in LANG_MAP.items():
            self.assertEqual(cls.name(), lang)

    def test_all_have_source_extensions(self) -> None:
        from languages import LANG_MAP
        for cls in LANG_MAP.values():
            exts = cls.source_extensions()
            self.assertTrue(len(exts) >= 1)
            for ext in exts:
                self.assertTrue(ext.startswith("."))

    def test_all_extractors_handle_empty_source(self) -> None:
        """All extractors should return empty lists for empty source."""
        from languages import LANG_MAP
        for _, cls in LANG_MAP.items():
            ext = cls("/tmp")  # type: ignore[abstract]
            sigs = ext.extract_signatures("/tmp/empty", b"")
            imps = ext.extract_imports("/tmp/empty", b"")
            self.assertEqual(sigs, [], f"{cls.__name__} returned non-empty sigs for empty source")
            self.assertEqual(imps, [], f"{cls.__name__} returned non-empty imps for empty source")

    def test_all_extractors_handle_junk_source(self) -> None:
        """All extractors should gracefully handle binary/garbage source."""
        from languages import LANG_MAP
        junk = b"\x00\x01\x02\xff\xfe\xfd\xfc\x00\x01\x02"
        for _, cls in LANG_MAP.items():
            ext = cls("/tmp")  # type: ignore[abstract]
            sigs = ext.extract_signatures("/tmp/garbage", junk)
            imps = ext.extract_imports("/tmp/garbage", junk)
            self.assertIsInstance(sigs, list)
            self.assertIsInstance(imps, list)

    def test_all_extractors_signatures_have_required_keys(self) -> None:
        """Every extractor returns signatures with the standard schema keys."""
        from languages import LANG_MAP
        required = {"type", "name", "signature", "visibility", "doc_comment",
                     "line_start", "line_end", "is_async", "is_unsafe", "generic_params"}
        source = b"# just a comment\n"
        for _, cls in LANG_MAP.items():
            ext = cls("/tmp")  # type: ignore[abstract]
            sigs = ext.extract_signatures("/tmp/test", source)
            for sig in sigs:
                missing = required - set(sig.keys())
                self.assertFalse(
                    missing,
                    f"{cls.__name__} missing keys: {missing} in {sig}"
                )

    def test_all_extractors_imports_have_required_keys(self) -> None:
        """Every extractor returns imports with the standard schema keys."""
        from languages import LANG_MAP
        required = {"import_text", "import_type", "line_start", "line_end"}
        source = b"# just a comment\n"
        for _, cls in LANG_MAP.items():
            ext = cls("/tmp")  # type: ignore[abstract]
            imps = ext.extract_imports("/tmp/test", source)
            for imp in imps:
                missing = required - set(imp.keys())
                self.assertFalse(
                    missing,
                    f"{cls.__name__} missing keys: {missing} in {imp}"
                )


if __name__ == "__main__":
    unittest.main()
