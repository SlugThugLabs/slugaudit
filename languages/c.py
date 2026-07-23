"""Tree-sitter C extractor — extracts signatures and imports from .c and .h files."""

import os
import re

from tree_sitter import Language, Parser
import tree_sitter_c as tsc

from .base import BaseExtractor
from typing import Any


class CExtractor(BaseExtractor):
    """Extractor for C source files using tree-sitter."""

    FN_DEF = "function_definition"
    DECLARATION = "declaration"
    STRUCT_SPEC = "struct_specifier"
    UNION_SPEC = "union_specifier"
    ENUM_SPEC = "enum_specifier"
    TYPE_DEF = "type_definition"
    PREPROC_INCLUDE = "preproc_include"
    COMMENT = "comment"

    @classmethod
    def name(cls) -> str:
        return "c"

    @classmethod
    def source_extensions(cls) -> set[str]:
        return {".c", ".h"}

    @property
    def parser(self) -> Any:
        if self._parser is None:
            c_lang = Language(tsc.language())
            p = Parser(c_lang)
            self._parser = p
        return self._parser

    def _handle_signature_node(self, cursor: Any, source_bytes: bytes, source_lines: list[str], signatures: list[Any], file_path: str) -> None:
        """Handle a single node during signature extraction."""
        node = cursor.node
        node_type = node.type

        if node_type == self.FN_DEF:
            sig = self._safe_extract(self._extract_fn, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.TYPE_DEF:
            sig = self._safe_extract(self._extract_typedef, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type in (self.STRUCT_SPEC, self.UNION_SPEC, self.ENUM_SPEC):
            sig = self._safe_extract(self._extract_top_type, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.DECLARATION:
            sig = self._safe_extract(self._extract_declaration, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

    def _get_declarator_name(self, node: Any, source_bytes: bytes) -> str:
        """Extract the name from a declarator node."""
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "type_identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "pointer_declarator":
                return self._get_declarator_name(child, source_bytes)
            if child.type == "function_declarator":
                return self._get_declarator_name(child, source_bytes)
            if child.type == "array_declarator":
                return self._get_declarator_name(child, source_bytes)
        return "unnamed"

    def _get_fn_name(self, node: Any, source_bytes: bytes) -> str:
        """Get function name from a function_definition node."""
        for child in node.named_children:
            if child.type == "function_declarator":
                return self._get_declarator_name(child, source_bytes)
        return "unnamed"

    def _extract_fn(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = self._get_fn_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Check for static (visibility)
            visibility = ""
            for child in node.named_children:
                if child.type == "storage_class_specifier":
                    vis = self.collect_node_text(child, source_bytes).strip()
                    if vis in ("static", "extern"):
                        visibility = vis
                        break

            # Truncate body
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

    def _extract_declaration(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        """Extract struct, union, enum type declarations."""
        try:
            for child in node.named_children:
                child_type = child.type
                kind = None
                if child_type == self.STRUCT_SPEC:
                    kind = "struct"
                elif child_type == self.UNION_SPEC:
                    kind = "union"
                elif child_type == self.ENUM_SPEC:
                    kind = "enum"

                if kind:
                    name = self._get_declarator_name(child, source_bytes)
                    if name == "unnamed":
                        # Check if it has a name node directly
                        for c in child.named_children:
                            if c.type == "identifier":
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

    def _extract_top_type(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        """Extract a struct/union/enum at the top level (not inside a declaration)."""
        try:
            kind_map = {self.STRUCT_SPEC: "struct", self.UNION_SPEC: "union", self.ENUM_SPEC: "enum"}
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

    def _extract_typedef(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            sig_text = self.collect_node_text(node, source_bytes)
            # Extract the type alias name (last identifier in the typedef)
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

    # ── Import extraction ──────────────────────────────────────────────────

    def _handle_import_node(self, cursor: Any, source_bytes: bytes, imports: list[Any], file_path: str) -> None:
        """Handle a single node during import extraction."""
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

    def _classify_include(self, inc_text: str) -> str:
        """Classify a C include as internal or external.

        #include <system_header.h> → external
        #include "local_header.h"   → internal
        """
        if '"' in inc_text:
            return "internal"
        return "external"

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a C include to a file path.

        Handles:
            #include "foo.h"          → foo.h or path/to/foo.h
            #include "../bar/baz.h"   → relative path
        """
        # Extract the filename from quotes
        if '"' not in import_text:
            return None  # System includes stay external

        start = import_text.find('"')
        end = import_text.rfind('"')
        if start < 0 or end <= start:
            return None

        filename = import_text[start + 1:end]

        # Try relative to the source file
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

    # ── Risk pattern extraction ──────────────────────────────────────────

    def extract_risk_patterns(self, file_path: str, source_bytes: bytes) -> list[dict[str, Any]]:
        """Extract risky C patterns: gets, unbounded strcpy/sprintf, raw pointers."""
        text = source_bytes.decode("utf-8", errors="replace")

        # Filter out comment lines
        lines = text.split("\n")
        code_lines = [line for line in lines if not line.strip().startswith("//")
                      and not line.strip().startswith("/*") and not line.strip().startswith("*")]
        code_text = "\n".join(code_lines)

        counts: dict[str, int] = {}
        patterns = [
            (r'\bgets\s*\(', 'gets'),
            (r'\bstrcpy\s*\(', 'strcpy'),
            (r'\bstrcat\s*\(', 'strcat'),
            (r'\bsprintf\s*\(', 'sprintf'),
            (r'\bsystem\s*\(', 'system'),
            (r'\bmalloc\s*\(', 'malloc'),
            (r'\bfree\s*\(', 'free'),
        ]

        for pattern, name in patterns:
            matches = re.findall(pattern, code_text)
            if matches:
                counts[name] = len(matches)

        return [{"pattern_type": k, "count": v} for k, v in counts.items() if v > 0]


__all__ = ["CExtractor"]
