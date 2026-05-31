"""Tree-sitter Ruby extractor — extracts signatures and imports from .rb files."""

import os
import re
from typing import Optional

from tree_sitter import Language, Parser
import tree_sitter_ruby as tsruby

from .base import BaseExtractor


class RubyExtractor(BaseExtractor):
    """Extractor for Ruby source files using tree-sitter."""

    METHOD = "method"
    SINGLETON_METHOD = "singleton_method"
    CLASS = "class"
    MODULE = "module"
    SINGLETON_CLASS = "singleton_class"
    CALL = "call"
    ASSIGNMENT = "assignment"
    COMMENT = "comment"

    @classmethod
    def name(cls) -> str:
        return "ruby"

    @classmethod
    def source_extensions(cls) -> set:
        return {".rb", ".rake", ".gemspec"}

    @property
    def parser(self):
        if self._parser is None:
            ruby_lang = Language(tsruby.language())
            p = Parser(ruby_lang)
            self._parser = p
        return self._parser

    def extract_signatures(self, file_path: str, source_bytes: bytes) -> list[dict]:
        parser = self.get_parser()
        tree = parser.parse(source_bytes)
        root = tree.root_node
        source_lines = source_bytes.decode("utf-8", errors="replace").splitlines(keepends=True)

        signatures = []
        cursor = root.walk()
        self._walk_tree(cursor, source_bytes, source_lines, signatures)
        return signatures

    def _walk_tree(self, cursor, source_bytes, source_lines, signatures):
        node = cursor.node
        node_type = node.type

        if node_type == self.METHOD:
            sig = self._extract_method(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.SINGLETON_METHOD:
            sig = self._extract_method(node, source_bytes, source_lines)
            if sig:
                sig["type"] = "singleton_method"
                signatures.append(sig)

        elif node_type == self.CLASS:
            sig = self._extract_class_or_module(node, source_bytes, source_lines, "class")
            if sig:
                signatures.append(sig)

        elif node_type == self.MODULE:
            sig = self._extract_class_or_module(node, source_bytes, source_lines, "module")
            if sig:
                signatures.append(sig)

        elif node_type == self.SINGLETON_CLASS:
            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("\n")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip()
            signatures.append({
                "type": "singleton_class",
                "name": "<< self",
                "signature": sig_text[:500],
                "visibility": "",
                "doc_comment": "",
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            })

        # Recurse into children
        if cursor.goto_first_child():
            self._walk_tree(cursor, source_bytes, source_lines, signatures)
            while cursor.goto_next_sibling():
                self._walk_tree(cursor, source_bytes, source_lines, signatures)
            cursor.goto_parent()

    def _get_name(self, node, source_bytes) -> str:
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "constant":
                return self.collect_node_text(child, source_bytes).strip()
        return "unnamed"

    def _get_constant_name(self, node, source_bytes) -> str:
        """Get the class/module name from a class/module node."""
        for child in node.named_children:
            if child.type == "constant":
                return self.collect_node_text(child, source_bytes).strip()
            # Handle scoped constant: Foo::Bar
            if child.type == "scope_resolution":
                return self.collect_node_text(child, source_bytes)
        return "unnamed"

    def _extract_method(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            # Get method name (first identifier child)
            name = "unnamed"
            for child in node.named_children:
                if child.type == "identifier":
                    name = self.collect_node_text(child, source_bytes)
                    break

            sig_text = self.collect_node_text(node, source_bytes)

            # Truncate body
            # Ruby methods end with 'end'
            # Take just the first line or the signature part
            lines = sig_text.split("\n")
            sig_line = lines[0] if lines else sig_text
            if len(lines) > 1:
                sig_text = sig_line + " ... end"

            # Check for visibility modifiers by looking at preceding comments/calls
            visibility = ""

            return {
                "type": "method",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": "",
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_class_or_module(self, node, source_bytes, source_lines, kind: str) -> Optional[dict]:
        try:
            name = self._get_constant_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            lines = sig_text.split("\n")
            sig_line = lines[0] if lines else sig_text
            if len(lines) > 1:
                sig_text = sig_line + " ... end"

            return {
                "type": kind,
                "name": name,
                "signature": sig_text[:500],
                "visibility": "",
                "doc_comment": "",
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    # ── Import extraction ──────────────────────────────────────────────────

    def extract_imports(self, file_path: str, source_bytes: bytes) -> list[dict]:
        parser = self.get_parser()
        tree = parser.parse(source_bytes)
        root = tree.root_node

        imports = []
        cursor = root.walk()
        self._walk_imports(cursor, source_bytes, imports)

        return imports

    def _walk_imports(self, cursor, source_bytes, imports):
        node = cursor.node

        if node.type == self.CALL:
            imp_text = self.collect_node_text(node, source_bytes).strip()

            # Check if this is a require/include/extend/prepend/load call
            method_name = self._get_call_method_name(node, source_bytes)
            if method_name in ("require", "require_relative", "load", "autoload",
                                "include", "extend", "prepend"):
                imp_type = "internal" if method_name == "require_relative" else "external"
                imports.append({
                    "import_text": imp_text,
                    "import_type": imp_type,
                    "line_start": node.start_point[0] + 1,
                    "line_end": node.end_point[0] + 1,
                })

        if cursor.goto_first_child():
            self._walk_imports(cursor, source_bytes, imports)
            while cursor.goto_next_sibling():
                self._walk_imports(cursor, source_bytes, imports)
            cursor.goto_parent()

    def _get_call_method_name(self, node, source_bytes) -> Optional[str]:
        """Get the method name from a call node."""
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
        return None

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict) -> Optional[str]:
        """Resolve a Ruby require to a file path.
        
        require 'foo'          → foo.rb
        require_relative 'bar' → ./bar.rb
        require './baz'        → ./baz.rb
        """
        # Extract the argument string
        # require 'foo' → 'foo'
        # require "bar" → "bar"
        m = re.search(r"['\"]([^'\"]+)['\"]", import_text)
        if not m:
            return None

        path_arg = m.group(1)

        # Check if it's a require_relative (relative to source file)
        if import_text.startswith("require_relative"):
            src_dir = os.path.dirname(source_file)
            candidate = os.path.normpath(os.path.join(src_dir, path_arg))
            return self._try_rb_paths(candidate)

        # For require, check if the path exists relative to project root
        abspath = os.path.join(self.project_root, path_arg)
        abspath_rb = abspath + ".rb"
        if os.path.exists(abspath_rb) and os.path.isfile(abspath_rb):
            return path_arg + ".rb"

        # Check in lib/ directory
        lib_path = os.path.join("lib", path_arg)
        candidate = self._try_rb_paths(lib_path)
        if candidate:
            return candidate

        # Check in app/ directory (Rails convention)
        app_path = os.path.join("app", path_arg)
        candidate = self._try_rb_paths(app_path)
        if candidate:
            return candidate

        return None

    def _try_rb_paths(self, base_path: str) -> Optional[str]:
        """Try common Ruby file path conventions."""
        candidates = [
            base_path + ".rb",
            os.path.join(base_path, "init.rb"),
            base_path + ".rake",
            base_path + ".gemspec",
        ]
        for candidate in candidates:
            abspath = os.path.join(self.project_root, candidate)
            if os.path.exists(abspath) and os.path.isfile(abspath):
                return candidate
        return None


__all__ = ["RubyExtractor"]
