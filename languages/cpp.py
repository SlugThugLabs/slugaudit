"""Tree-sitter C++ extractor — extracts signatures and imports from .cpp, .hpp, .cc, .cxx files."""

import os
from typing import Optional

from tree_sitter import Language, Parser
import tree_sitter_cpp as tscpp

from .base import BaseExtractor


class CppExtractor(BaseExtractor):
    """Extractor for C++ source files using tree-sitter."""

    FN_DEF = "function_definition"
    DECLARATION = "declaration"
    CLASS_SPEC = "class_specifier"
    STRUCT_SPEC = "struct_specifier"
    UNION_SPEC = "union_specifier"
    ENUM_SPEC = "enum_specifier"
    TYPE_DEF = "type_definition"
    TEMPLATE_DECL = "template_declaration"
    NAMESPACE_DEF = "namespace_definition"
    PREPROC_INCLUDE = "preproc_include"
    COMMENT = "comment"

    @classmethod
    def name(cls) -> str:
        return "cpp"

    @classmethod
    def source_extensions(cls) -> set:
        return {".cpp", ".hpp", ".cc", ".cxx", ".hxx", ".hh", ".ixx", ".tpp"}

    @property
    def parser(self):
        if self._parser is None:
            cpp_lang = Language(tscpp.language())
            p = Parser(cpp_lang)
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

        if node_type == self.FN_DEF:
            sig = self._extract_fn(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.TYPE_DEF:
            sig = self._extract_typedef(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type in (self.CLASS_SPEC, self.STRUCT_SPEC, self.UNION_SPEC, self.ENUM_SPEC):
            sig = self._extract_top_type(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.DECLARATION:
            sig = self._extract_type(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.TEMPLATE_DECL:
            sig = self._extract_template(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.NAMESPACE_DEF:
            sig = self._extract_namespace(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        # Recurse into children
        if cursor.goto_first_child():
            self._walk_tree(cursor, source_bytes, source_lines, signatures)
            while cursor.goto_next_sibling():
                self._walk_tree(cursor, source_bytes, source_lines, signatures)
            cursor.goto_parent()

    def _get_declarator_name(self, node, source_bytes) -> str:
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "field_identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "type_identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "qualified_identifier":
                for c in child.named_children:
                    if c.type == "identifier":
                        return self.collect_node_text(c, source_bytes).strip()
                    if c.type == "type_identifier":
                        return self.collect_node_text(c, source_bytes).strip()
            if child.type in ("pointer_declarator", "function_declarator", "array_declarator",
                               "reference_declarator"):
                return self._get_declarator_name(child, source_bytes)
        return "unnamed"

    def _get_fn_name(self, node, source_bytes) -> str:
        for child in node.named_children:
            if child.type == "function_declarator":
                return self._get_declarator_name(child, source_bytes)
            if child.type == "qualified_identifier":
                for c in child.named_children:
                    if c.type == "identifier":
                        return self.collect_node_text(c, source_bytes).strip()
        return "unnamed"

    def _extract_fn(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            name = self._get_fn_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            visibility = ""
            for child in node.named_children:
                if child.type == "storage_class_specifier":
                    vis = self.collect_node_text(child, source_bytes).strip()
                    if vis in ("static", "extern", "inline", "virtual"):
                        visibility = vis
                        break

            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            doc = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": "function",
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

    def _extract_type(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            for child in node.named_children:
                child_type = child.type
                kind = None
                if child_type == self.CLASS_SPEC:
                    kind = "class"
                elif child_type == self.STRUCT_SPEC:
                    kind = "struct"
                elif child_type == self.UNION_SPEC:
                    kind = "union"
                elif child_type == self.ENUM_SPEC:
                    kind = "enum"

                if kind:
                    name = "unnamed"
                    for c in child.named_children:
                        if c.type == "identifier":
                            name = self.collect_node_text(c, source_bytes)
                            break
                        if c.type == "type_identifier":
                            name = self.collect_node_text(c, source_bytes)
                            break
                    if name == "unnamed":
                        continue

                    sig_text = self.collect_node_text(child, source_bytes)
                    brace_idx = sig_text.find("{")
                    if brace_idx >= 0:
                        sig_text = sig_text[:brace_idx].strip() + " { ... }"

                    doc = self._get_doc_comment_above(node, source_bytes, source_lines)

                    return {
                        "type": kind,
                        "name": name,
                        "signature": sig_text[:500],
                        "visibility": "",
                        "doc_comment": doc,
                        "line_start": node.start_point[0] + 1,
                        "line_end": node.end_point[0] + 1,
                        "is_async": False,
                        "is_unsafe": False,
                        "generic_params": "",
                    }
            return None
        except Exception:
            return None

    def _extract_top_type(self, node, source_bytes, source_lines) -> Optional[dict]:
        """Extract a class/struct/union/enum at the top level."""
        try:
            kind_map = {
                self.CLASS_SPEC: "class", self.STRUCT_SPEC: "struct",
                self.UNION_SPEC: "union", self.ENUM_SPEC: "enum"
            }
            kind = kind_map.get(node.type, "type")
            
            name = "unnamed"
            for c in node.named_children:
                if c.type == "identifier":
                    name = self.collect_node_text(c, source_bytes)
                    break
                if c.type == "type_identifier":
                    name = self.collect_node_text(c, source_bytes)
                    break
            if name == "unnamed":
                return None
            
            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"
            
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

    def _extract_typedef(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            sig_text = self.collect_node_text(node, source_bytes)
            name = "unnamed"
            for child in reversed(node.named_children):
                if child.type == "type_identifier":
                    name = self.collect_node_text(child, source_bytes)
                    break
                n = self._get_declarator_name(child, source_bytes)
                if n != "unnamed":
                    name = n
                    break

            doc = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": "typedef",
                "name": name,
                "signature": sig_text[:500],
                "visibility": "",
                "doc_comment": doc,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_template(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            # Extract the declaration inside the template
            for child in node.named_children:
                if child.type == self.FN_DEF:
                    sig = self._extract_fn(child, source_bytes, source_lines)
                    if sig:
                        sig["type"] = "template_fn"
                        # Prepend template parameters to signature
                        template_params = ""
                        for c in node.named_children:
                            if c.type == "template_parameter_list":
                                template_params = self.collect_node_text(c, source_bytes)
                                break
                        if template_params:
                            sig["signature"] = f"template {template_params} {sig['signature']}"[:500]
                        return sig

                if child.type == self.DECLARATION:
                    inner = self._extract_type(child, source_bytes, source_lines)
                    if inner:
                        inner["type"] = f"template_{inner['type']}"
                        template_params = ""
                        for c in node.named_children:
                            if c.type == "template_parameter_list":
                                template_params = self.collect_node_text(c, source_bytes)
                                break
                        if template_params:
                            inner["signature"] = f"template {template_params} {inner['signature']}"[:500]
                        return inner
            return None
        except Exception:
            return None

    def _extract_namespace(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            name = "unnamed"
            for child in node.named_children:
                if child.type == "identifier":
                    name = self.collect_node_text(child, source_bytes)
                    break
                if child.type == "namespace_identifier":
                    name = self.collect_node_text(child, source_bytes)
                    break

            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            return {
                "type": "namespace",
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

        if node.type == self.PREPROC_INCLUDE:
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imp_type = self._classify_include(imp_text)
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

    def _classify_include(self, inc_text: str) -> str:
        if '"' in inc_text:
            return "internal"
        return "external"

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict) -> Optional[str]:
        """Resolve a C++ include to a file path."""
        if '"' not in import_text:
            return None

        start = import_text.find('"')
        end = import_text.rfind('"')
        if start < 0 or end <= start:
            return None

        filename = import_text[start + 1:end]

        # Try relative to source file
        src_dir = os.path.dirname(source_file)
        candidate = os.path.normpath(os.path.join(src_dir, filename))
        abspath = os.path.join(self.project_root, candidate)
        if os.path.exists(abspath) and os.path.isfile(abspath):
            return candidate

        # Try relative to project root
        abspath = os.path.join(self.project_root, filename)
        if os.path.exists(abspath) and os.path.isfile(abspath):
            return filename

        # Try common include directories
        for base in ("include", "src", "lib"):
            candidate = os.path.join(base, filename)
            abspath = os.path.join(self.project_root, candidate)
            if os.path.exists(abspath) and os.path.isfile(abspath):
                return candidate

        return None


__all__ = ["CppExtractor"]
