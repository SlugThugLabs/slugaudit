"""Tree-sitter TypeScript/JavaScript extractor."""

import os
from typing import Optional

from tree_sitter import Language, Parser
import tree_sitter_typescript as tstypescript

from .base import BaseExtractor


class TypeScriptExtractor(BaseExtractor):
    """Extractor for TypeScript/JavaScript source files using tree-sitter."""

    FN_DECL = "function_declaration"
    GENERATOR_FN = "generator_function_declaration"
    METHOD = "method_definition"
    CLASS = "class_declaration"
    INTERFACE = "interface_declaration"
    TYPE_ALIAS = "type_alias_declaration"
    ENUM = "enum_declaration"
    VARIABLE = "variable_declaration"
    ARROW = "arrow_function"
    EXPORT = "export_statement"
    IMPORT = "import_statement"
    COMMENT = "comment"

    @classmethod
    def name(cls) -> str:
        return "typescript"

    @classmethod
    def source_extensions(cls) -> set:
        return {".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"}

    @property
    def parser(self):
        if self._parser is None:
            # Use TypeScript grammar (also handles TSX)
            ts_lang = Language(tstypescript.language_typescript())
            p = Parser(ts_lang)
            self._parser = p
        return self._parser

    def extract_signatures(self, file_path: str, source_bytes: bytes) -> list[dict]:
        parser = self.get_parser()
        tree = parser.parse(source_bytes)
        root = tree.root_node
        source_lines = source_bytes.decode("utf-8", errors="replace").splitlines(keepends=True)

        signatures = []
        cursor = root.walk()
        self._walk_tree(cursor, source_bytes, source_lines, signatures, file_path)
        return signatures

    def _walk_tree(self, cursor, source_bytes, source_lines, signatures, file_path):
        node = cursor.node
        node_type = node.type

        if node_type == "export_statement":
            # The exported declaration is inside
            for child in node.named_children:
                self._extract_declaration(child, source_bytes, source_lines, signatures, exported=True)
                # Recurse into this child's subtree
                cursor2 = child.walk()
                if cursor2.goto_first_child():
                    self._walk_tree(cursor2, source_bytes, source_lines, signatures, file_path)
                    while cursor2.goto_next_sibling():
                        self._walk_tree(cursor2, source_bytes, source_lines, signatures, file_path)
        else:
            self._extract_declaration(node, source_bytes, source_lines, signatures, exported=False)
            # Recurse into children
            if cursor.goto_first_child():
                self._walk_tree(cursor, source_bytes, source_lines, signatures, file_path)
                while cursor.goto_next_sibling():
                    self._walk_tree(cursor, source_bytes, source_lines, signatures, file_path)
                cursor.goto_parent()

    def _extract_declaration(self, node, source_bytes, source_lines, signatures, exported: bool):
        """Extract a signature from a declaration node, if applicable."""
        node_type = node.type

        if node_type == self.FN_DECL:
            sig = self._extract_fn(node, source_bytes, source_lines, exported)
        elif node_type == self.GENERATOR_FN:
            sig = self._extract_fn(node, source_bytes, source_lines, exported)
        elif node_type == self.CLASS:
            sig = self._extract_class(node, source_bytes, source_lines, exported)
        elif node_type == self.INTERFACE:
            sig = self._extract_interface(node, source_bytes, source_lines, exported)
        elif node_type == self.TYPE_ALIAS:
            sig = self._extract_type_alias(node, source_bytes, source_lines, exported)
        elif node_type == self.ENUM:
            sig = self._extract_enum(node, source_bytes, source_lines, exported)
        elif node_type == self.VARIABLE:
            sig = self._extract_variable(node, source_bytes, source_lines, exported)
        else:
            sig = None

        if sig:
            signatures.append(sig)

    def _get_name(self, node, source_bytes) -> str:
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "type_identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "property_identifier":
                return self.collect_node_text(child, source_bytes).strip()
        return "unnamed"

    def _collect_jsdoc(self, node, source_bytes, source_lines) -> str:
        """Collect JSDoc comment above a node."""
        if node.start_point[0] == 0:
            return ""
        docs = []
        line_idx = node.start_point[0] - 1
        while line_idx >= 0:
            line = source_lines[line_idx].strip()
            if line.startswith("/**") or line.startswith("* ") or line.startswith("*/") or line.startswith("//"):
                docs.insert(0, line.lstrip("/*").lstrip("*").lstrip(" ").rstrip("*/").strip())
                line_idx -= 1
            elif line == "":
                line_idx -= 1
            else:
                break
        return " ".join(docs)

    def _extract_fn(self, node, source_bytes, source_lines, exported: bool) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            is_async = any(child.type == "async" for child in node.children)
            is_generator = node.type == self.GENERATOR_FN

            # Truncate body
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            doc = self._collect_jsdoc(node, source_bytes, source_lines)
            visibility = "export" if exported else ""

            return {
                "type": "fn",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": is_async,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_class(self, node, source_bytes, source_lines, exported: bool) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            doc = self._collect_jsdoc(node, source_bytes, source_lines)
            visibility = "export" if exported else ""

            return {
                "type": "class",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_interface(self, node, source_bytes, source_lines, exported: bool) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            generic_params = ""
            for child in node.children:
                if child.type == "type_parameters":
                    generic_params = self.collect_node_text(child, source_bytes)
                    break

            doc = self._collect_jsdoc(node, source_bytes, source_lines)
            visibility = "export" if exported else ""

            return {
                "type": "interface",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": generic_params,
            }
        except Exception:
            return None

    def _extract_type_alias(self, node, source_bytes, source_lines, exported: bool) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)
            doc = self._collect_jsdoc(node, source_bytes, source_lines)
            visibility = "export" if exported else ""

            return {
                "type": "type_alias",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_enum(self, node, source_bytes, source_lines, exported: bool) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"
            doc = self._collect_jsdoc(node, source_bytes, source_lines)
            visibility = "export" if exported else ""

            return {
                "type": "enum",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_variable(self, node, source_bytes, source_lines, exported: bool) -> Optional[dict]:
        try:
            # const x = ...; or let x = ...;
            declarators = []
            for child in node.children:
                if child.type == "variable_declarator":
                    declarators.append(child)

            if not declarators:
                return None

            decl = declarators[0]
            name_node = None
            for child in decl.named_children:
                if child.type == "identifier":
                    name_node = child
                    break

            if not name_node:
                return None

            name = self.collect_node_text(name_node, source_bytes)

            # Check if it's a const or let
            kind = "const"
            for child in node.children:
                if child.type == "let":
                    kind = "let"
                elif child.type == "var":
                    kind = "var"

            sig_text = f"{kind} {name}"
            doc = self._collect_jsdoc(node, source_bytes, source_lines)
            visibility = "export" if exported else ""

            return {
                "type": "variable",
                "name": name,
                "signature": sig_text,
                "visibility": visibility,
                "doc_comment": doc,
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
        self._walk_imports(cursor, source_bytes, imports, file_path)

        return imports

    def _walk_imports(self, cursor, source_bytes, imports, file_path):
        node = cursor.node

        if node.type == self.IMPORT:
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imp_type = self._classify_import(imp_text)
            imports.append({
                "import_text": imp_text,
                "import_type": imp_type,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
            })

        if cursor.goto_first_child():
            self._walk_imports(cursor, source_bytes, imports, file_path)
            while cursor.goto_next_sibling():
                self._walk_imports(cursor, source_bytes, imports, file_path)
            cursor.goto_parent()

    def _classify_import(self, imp_text: str) -> str:
        """Classify a TS/JS import as internal or external."""
        # Extract the module specifier from import declaration
        # import ... from 'module'
        # import 'module'
        import re
        m = re.search(r"from\s+['\"]([^'\"]+)['\"]", imp_text)
        if not m:
            # Side-effect import: import 'module'
            m = re.search(r"import\s+['\"]([^'\"]+)['\"]", imp_text)
        if not m:
            return "external"

        module = m.group(1)

        # Relative imports start with ./
        if module.startswith("./"):
            return "internal"
        if module.startswith("../"):
            return "internal"

        # Everything else is external (npm package, etc.)
        return "external"

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict) -> Optional[str]:
        """Resolve a TypeScript import to a file path.

        import { x } from './foo'  →  ./foo.ts, ./foo.tsx, ./foo/index.ts
        import { y } from '../bar' →  ../bar.ts
        """
        import re
        m = re.search(r"from\s+['\"]([^'\"]+)['\"]", import_text)
        if not m:
            return None

        module = m.group(1)

        # Only resolve relative imports
        if not module.startswith("./"):
            return None

        src_dir = os.path.dirname(source_file)
        base = os.path.normpath(os.path.join(src_dir, module))

        return self._try_ts_paths(base)

    def _try_ts_paths(self, base_path: str) -> Optional[str]:
        """Try common TypeScript file path conventions."""
        candidates = [
            base_path + ".ts",
            base_path + ".tsx",
            base_path + ".js",
            base_path + ".jsx",
            base_path + "/index.ts",
            base_path + "/index.tsx",
            base_path + "/index.js",
            base_path + "/index.jsx",
        ]
        for candidate in candidates:
            abspath = os.path.join(self.project_root, candidate)
            if os.path.exists(abspath) and os.path.isfile(abspath):
                return candidate
        return None


__all__ = ["TypeScriptExtractor"]
