"""Import classification/resolution correctness for all 8 languages.

Complements tests/test_extractors.py's TestRustImportResolution — this file
covers the other 7, most of which had real classification bugs found and
fixed in this pass: Python, Java, and Ruby classified real local imports as
external unconditionally (so resolve_import's working logic never ran), and
Go's resolver was dead code entirely (no go.mod-based module-path handling,
plus a broken fixed-segment-count heuristic). See each language's
_classify_import docstring for the specific rationale.
"""

import os
import tempfile
import unittest

from languages import (
    CExtractor,
    CppExtractor,
    GoExtractor,
    JavaExtractor,
    PythonExtractor,
    RubyExtractor,
    TypeScriptExtractor,
)


def _write(root: str, relpath: str, content: str) -> None:
    full = os.path.join(root, relpath)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as fh:
        fh.write(content)


class TestPythonImportResolution(unittest.TestCase):
    """Regression coverage for the top-level-absolute-import bug."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = PythonExtractor(self.tmpdir)

    def test_top_level_absolute_import_of_local_module_is_internal(self) -> None:
        _write(self.tmpdir, "b.py", "def helper():\n    return 1\n")
        imps = self.ext.extract_imports("a.py", b"from b import helper\n")
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(
            self.ext.resolve_import(imps[0]["import_text"], "a.py", {}), "b.py"
        )

    def test_absolute_import_of_local_package_is_internal(self) -> None:
        _write(self.tmpdir, "pkg/__init__.py", "")
        _write(self.tmpdir, "pkg/mod.py", "X = 1\n")
        imps = self.ext.extract_imports("a.py", b"from pkg.mod import X\n")
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(
            self.ext.resolve_import(imps[0]["import_text"], "a.py", {}), "pkg/mod.py"
        )

    def test_third_party_import_stays_external(self) -> None:
        # No local "requests.py"/"requests/" exists in the project.
        imps = self.ext.extract_imports("a.py", b"import requests\n")
        self.assertEqual(imps[0]["import_type"], "external")

    def test_relative_import_is_internal_regardless_of_resolution(self) -> None:
        _write(self.tmpdir, "pkg/__init__.py", "")
        imps = self.ext.extract_imports("pkg/a.py", b"from . import missing_sibling\n")
        self.assertEqual(imps[0]["import_type"], "internal")


class TestGoImportResolution(unittest.TestCase):
    """Regression coverage: this resolver was 100% dead code before this fix."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()

    def test_no_go_mod_means_nothing_resolves(self) -> None:
        ext = GoExtractor(self.tmpdir)
        imps = ext.extract_imports("main.go", b'package main\n\nimport "fmt"\n')
        self.assertEqual(imps[0]["import_type"], "external")

    def test_same_module_import_is_internal_and_resolves(self) -> None:
        _write(self.tmpdir, "go.mod", "module github.com/user/project\n\ngo 1.21\n")
        _write(self.tmpdir, "pkg/utils/helper.go", "package utils\n\nfunc Helper() int { return 1 }\n")
        ext = GoExtractor(self.tmpdir)
        source = (
            b'package main\n\nimport (\n\t"fmt"\n\t"github.com/user/project/pkg/utils"\n)\n'
        )
        imps = ext.extract_imports("main.go", source)
        by_text = {i["import_text"]: i for i in imps}
        self.assertEqual(by_text['"fmt"']["import_type"], "external")
        internal = by_text['"github.com/user/project/pkg/utils"']
        self.assertEqual(internal["import_type"], "internal")
        self.assertEqual(
            ext.resolve_import(internal["import_text"], "main.go", {}),
            "pkg/utils/helper.go",
        )

    def test_third_party_import_with_go_mod_present_stays_external(self) -> None:
        _write(self.tmpdir, "go.mod", "module github.com/user/project\n\ngo 1.21\n")
        ext = GoExtractor(self.tmpdir)
        imps = ext.extract_imports("main.go", b'package main\n\nimport "github.com/other/lib"\n')
        self.assertEqual(imps[0]["import_type"], "external")

    def test_module_root_package_itself_resolves(self) -> None:
        _write(self.tmpdir, "go.mod", "module example.com/app\n\ngo 1.21\n")
        _write(self.tmpdir, "root.go", "package app\n\nfunc Root() {}\n")
        ext = GoExtractor(self.tmpdir)
        resolved = ext.resolve_import('"example.com/app"', "cmd/main.go", {})
        self.assertEqual(resolved, "root.go")


class TestJavaImportResolution(unittest.TestCase):
    """Regression coverage: classification defaulted every non-JDK import external."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = JavaExtractor(self.tmpdir)

    def test_local_class_import_is_internal(self) -> None:
        _write(self.tmpdir, "com/example/util/StringUtils.java",
               "package com.example.util;\npublic class StringUtils {}\n")
        source = b"package com.example;\n\nimport com.example.util.StringUtils;\n\nclass Main {}\n"
        imps = self.ext.extract_imports("com/example/Main.java", source)
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(
            self.ext.resolve_import(imps[0]["import_text"], "com/example/Main.java", {}),
            "com/example/util/StringUtils.java",
        )

    def test_jdk_import_stays_external_without_filesystem_check(self) -> None:
        imps = self.ext.extract_imports("Main.java", b"import java.util.List;\n")
        self.assertEqual(imps[0]["import_type"], "external")

    def test_unresolvable_third_party_import_stays_external(self) -> None:
        imps = self.ext.extract_imports("Main.java", b"import com.google.common.collect.Lists;\n")
        self.assertEqual(imps[0]["import_type"], "external")


class TestRubyImportResolution(unittest.TestCase):
    """Regression coverage: only require_relative was ever classified internal."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = RubyExtractor(self.tmpdir)

    def test_require_relative_is_internal(self) -> None:
        _write(self.tmpdir, "lib/helper.rb", "def helper\n  1\nend\n")
        imps = self.ext.extract_imports("lib/app.rb", b"require_relative 'helper'\n")
        self.assertEqual(imps[0]["import_type"], "internal")

    def test_plain_require_of_local_lib_file_is_internal(self) -> None:
        _write(self.tmpdir, "lib/helper.rb", "def helper\n  1\nend\n")
        imps = self.ext.extract_imports("app.rb", b"require 'helper'\n")
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(
            self.ext.resolve_import(imps[0]["import_text"], "app.rb", {}), "lib/helper.rb"
        )

    def test_plain_require_of_a_gem_stays_external(self) -> None:
        imps = self.ext.extract_imports("app.rb", b"require 'json'\n")
        self.assertEqual(imps[0]["import_type"], "external")

    def test_include_of_a_module_constant_stays_external(self) -> None:
        # `include Comparable` has no quoted path at all for resolve_import
        # to find — it references an already-loaded constant, not a file.
        imps = self.ext.extract_imports("app.rb", b"class Foo\n  include Comparable\nend\n")
        self.assertEqual(imps[0]["import_type"], "external")


class TestCImportResolution(unittest.TestCase):
    """Classification here is unambiguous by syntax (quote vs angle-bracket);
    this just confirms resolution actually works, not only classification."""

    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = CExtractor(self.tmpdir)

    def test_quoted_include_of_local_header_resolves(self) -> None:
        _write(self.tmpdir, "foo.h", "#ifndef FOO_H\n#define FOO_H\n#endif\n")
        imps = self.ext.extract_imports("main.c", b'#include "foo.h"\n')
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(self.ext.resolve_import(imps[0]["import_text"], "main.c", {}), "foo.h")

    def test_angle_bracket_include_stays_external_even_if_same_name_exists_locally(self) -> None:
        # A local stdio.h would be a very strange thing to have, but the
        # point is angle brackets mean "system header" by C's own syntax
        # regardless of what happens to exist on disk.
        _write(self.tmpdir, "stdio.h", "// not the real one\n")
        imps = self.ext.extract_imports("main.c", b"#include <stdio.h>\n")
        self.assertEqual(imps[0]["import_type"], "external")


class TestCppImportResolution(unittest.TestCase):
    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = CppExtractor(self.tmpdir)

    def test_quoted_include_of_local_header_resolves(self) -> None:
        _write(self.tmpdir, "include/foo.hpp", "#pragma once\n")
        imps = self.ext.extract_imports("src/main.cpp", b'#include "foo.hpp"\n')
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(
            self.ext.resolve_import(imps[0]["import_text"], "src/main.cpp", {}), "include/foo.hpp"
        )

    def test_angle_bracket_include_stays_external(self) -> None:
        imps = self.ext.extract_imports("main.cpp", b"#include <vector>\n")
        self.assertEqual(imps[0]["import_type"], "external")


class TestTypeScriptImportResolution(unittest.TestCase):
    def setUp(self) -> None:
        self.tmpdir = tempfile.mkdtemp()
        self.ext = TypeScriptExtractor(self.tmpdir)

    def test_relative_import_resolves(self) -> None:
        _write(self.tmpdir, "utils.ts", "export function helper() { return 1; }\n")
        imps = self.ext.extract_imports("index.ts", b"import { helper } from './utils';\n")
        self.assertEqual(imps[0]["import_type"], "internal")
        self.assertEqual(
            self.ext.resolve_import(imps[0]["import_text"], "index.ts", {}), "utils.ts"
        )

    def test_bare_specifier_stays_external_even_if_same_name_file_exists(self) -> None:
        # Bare specifiers are npm packages by JS/TS convention regardless of
        # what happens to exist locally with a matching name.
        _write(self.tmpdir, "react.ts", "// decoy\n")
        imps = self.ext.extract_imports("index.ts", b"import React from 'react';\n")
        self.assertEqual(imps[0]["import_type"], "external")


if __name__ == "__main__":
    unittest.main()
