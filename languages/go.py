"""Tree-sitter Go extractor — extracts signatures and imports from .go files."""

import os
import re

from tree_sitter import Language, Parser
import tree_sitter_go as tsgo

from .base import BaseExtractor
from typing import Any


class GoExtractor(BaseExtractor):
    """Extractor for Go source files using tree-sitter."""

    FN_DECL = "function_declaration"
    METHOD_DECL = "method_declaration"
    TYPE_DECL = "type_declaration"
    TYPE_SPEC = "type_spec"
    STRUCT_TYPE = "struct_type"
    INTERFACE_TYPE = "interface_type"
    IMPORT_DECL = "import_declaration"
    IMPORT_SPEC = "import_spec"
    COMMENT = "comment"

    @classmethod
    def name(cls) -> str:
        return "go"

    @classmethod
    def source_extensions(cls) -> set[str]:
        return {".go"}

    @property
    def parser(self) -> Any:
        if self._parser is None:
            go_lang = Language(tsgo.language())
            p = Parser(go_lang)
            self._parser = p
        return self._parser

    def _handle_signature_node(self, cursor: Any, source_bytes: bytes, source_lines: list[str], signatures: list[Any], file_path: str) -> None:
        """Handle a single node during signature extraction."""
        node = cursor.node
        node_type = node.type

        if node_type == self.FN_DECL:
            sig = self._safe_extract(self._extract_fn, node, source_bytes, source_lines, "function")
            if sig:
                signatures.append(sig)

        elif node_type == self.METHOD_DECL:
            sig = self._safe_extract(self._extract_fn, node, source_bytes, source_lines, "method")
            if sig:
                signatures.append(sig)

        elif node_type == self.TYPE_DECL:
            for child in node.named_children:
                if child.type == self.TYPE_SPEC:
                    sig = self._safe_extract(self._extract_type_spec, child, source_bytes, source_lines)
                    if sig:
                        signatures.append(sig)

    def _get_name(self, node: Any, source_bytes: bytes) -> str:
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "type_identifier":
                return self.collect_node_text(child, source_bytes).strip()
            if child.type == "field_identifier":
                return self.collect_node_text(child, source_bytes).strip()
        return "unnamed"

    def _is_exported(self, name: str) -> bool:
        """In Go, exported names start with a capital letter."""
        return bool(name) and name[0].isupper()

    def _extract_fn(self, node: Any, source_bytes: bytes, source_lines: list[str], kind: str) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Truncate body
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            visibility = "exported" if self._is_exported(name) else ""

            # Get doc comment
            doc = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": kind,
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

    def _extract_type_spec(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Determine kind from the type value
            kind = "type"
            body = None
            for child in node.named_children:
                if child.type == self.STRUCT_TYPE:
                    kind = "struct"
                    body = child
                    break
                elif child.type == self.INTERFACE_TYPE:
                    kind = "interface"
                    body = child
                    break

            # Truncate body
            if body:
                brace_idx = sig_text.find("{")
                if brace_idx >= 0:
                    sig_text = sig_text[:brace_idx].strip() + " { ... }"

            visibility = "exported" if self._is_exported(name) else ""
            doc = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": kind,
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

    # ── Import extraction ──────────────────────────────────────────────────

    def _handle_import_node(self, cursor: Any, source_bytes: bytes, imports: list[Any], file_path: str) -> None:
        """Handle a single node during import extraction."""
        node = cursor.node

        if node.type == self.IMPORT_SPEC:
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imports.append({
                "import_text": imp_text,
                "import_type": "external",  # Go imports are external by default
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
            })

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a Go import to a local file path.

        Go imports use full module paths, so only relative imports (which Go
        doesn't really have in the same way) would resolve. We check for
        same-package imports by looking for files in the same directory.
        """
        # Go doesn't have relative imports in the traditional sense.
        # Internal packages within the same module are referenced by full path.
        # We can check if the import path matches a local directory.
        imp = import_text.strip().strip('"')

        # Skip standard library imports (no domain/org prefix)
        if "." not in imp and "/" not in imp:
            return None

        # Extract the last segment as a potential package path
        # e.g., github.com/user/project/pkg/subpkg → pkg/subpkg
        parts = imp.split("/")
        # Look for the module root in the project by checking if any path segment
        # matches a directory name from the project root
        mod_path = "/".join(parts[3:]) if len(parts) > 3 else "/".join(parts[1:])

        if not mod_path:
            return None

        candidate = mod_path
        # Check if this path exists directly
        abspath = os.path.join(self.project_root, candidate)
        if os.path.isdir(abspath):
            # Return the directory itself as the target for this package
            return candidate

        return None

    # ── Risk pattern extraction ──────────────────────────────────────────

    def extract_risk_patterns(self, file_path: str, source_bytes: bytes) -> list[dict[str, Any]]:
        """Extract risky Go patterns: ignored errors, panic, unsafe.Pointer."""
        text = source_bytes.decode("utf-8", errors="replace")

        # Filter out comment lines
        lines = text.split("\n")
        code_lines = [line for line in lines if not line.strip().startswith("//")]
        code_text = "\n".join(code_lines)

        counts: dict[str, int] = {}
        patterns = [
            (r'_,\s*(?:err|error)\s*[:=]', 'ignored_errors'),
            (r'\bpanic\s*\(', 'panic'),
            (r'\bunsafe\.Pointer\s*\(', 'unsafe_pointer'),
            (r'\bgo\s+func\s*\(', 'anonymous_goroutine'),
        ]

        for pattern, name in patterns:
            matches = re.findall(pattern, code_text)
            if matches:
                counts[name] = len(matches)

        return [{"pattern_type": k, "count": v} for k, v in counts.items() if v > 0]


__all__ = ["GoExtractor"]
