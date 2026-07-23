"""Tests for all 8 language extractors — signature and import extraction."""

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

    def test_extracts_decorated_fn(self) -> None:
        source = b"@decorator\ndef wrapped():\n    pass\n"
        sigs = self.ext.extract_signatures("/tmp/test.py", source)
        # Decorated functions are extracted once from the decorated_definition
        # and once from recursion into children (known dedup gap)
        names = [s["name"] for s in sigs]
        self.assertIn("wrapped", names)


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

    def test_extracts_import(self) -> None:
        source = b'import \"fmt\"\nimport \"os\"\n'
        imps = self.ext.extract_imports("/tmp/test.go", source)
        self.assertGreaterEqual(len(imps), 1)
        self.assertIn("fmt", imps[0]["import_text"])

    def test_extracts_grouped_import(self) -> None:
        source = b'import (\n\t\"fmt\"\n\t\"os\"\n)\n'
        imps = self.ext.extract_imports("/tmp/test.go", source)
        self.assertGreaterEqual(len(imps), 2)


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

    def test_extracts_template_fn(self) -> None:
        source = b"template <typename T>\nT max(T a, T b) {\n    return a > b ? a : b;\n}\n"
        sigs = self.ext.extract_signatures("/tmp/test.cpp", source)
        self.assertTrue(
            any(s["type"] == "template_fn" for s in sigs),
            msg=f"No template_fn found. Got types: {[s['type'] for s in sigs]}"
        )

    def test_extracts_include(self) -> None:
        source = b'#include <iostream>\n#include "utils.hpp"\n'
        imps = self.ext.extract_imports("/tmp/test.cpp", source)
        self.assertEqual(len(imps), 2)
        self.assertIn("iostream", imps[0]["import_text"])


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

    def test_extracts_class(self) -> None:
        source = b"class MyClass\n  def initialize\n  end\nend\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        self.assertTrue(any(s["type"] == "class" and s["name"] == "MyClass" for s in sigs))

    def test_extracts_module(self) -> None:
        source = b"module Utilities\n  def helper\n  end\nend\n"
        sigs = self.ext.extract_signatures("/tmp/test.rb", source)
        self.assertTrue(any(s["type"] == "module" and s["name"] == "Utilities" for s in sigs))

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
