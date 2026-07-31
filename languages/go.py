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

    def __init__(self, project_root: str) -> None:
        super().__init__(project_root)
        # Declared module path from go.mod, cached per sync like Rust's
        # crate_map — an import is internal iff it's rooted at this path.
        self._module_path: str | None = None
        self._module_path_loaded = False

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
            imp_type = self._classify_import(imp_text, file_path)
            imports.append({
                "import_text": imp_text,
                "import_type": imp_type,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
            })

    def _classify_import(self, imp_text: str, file_path: str) -> str:
        """Classify a Go import as internal or external.

        Go has no relative-import syntax to lean on the way Python/JS do —
        every import is a full module path, so "is this internal" can only
        be answered by checking whether it's rooted at *this project's own*
        module path (declared in go.mod). Without a go.mod, there's no
        reliable way to tell a same-module import from a third-party one, so
        everything stays external rather than guessing.
        """
        if self.resolve_import(imp_text, file_path, {}) is not None:
            return "internal"
        return "external"

    # ── Module path (go.mod) ────────────────────────────────────────────────

    @property
    def module_path(self) -> str | None:
        """This project's own module path, e.g. "github.com/user/project"."""
        if not self._module_path_loaded:
            self._module_path = self._read_go_mod_module_path()
            self._module_path_loaded = True
        return self._module_path

    def _read_go_mod_module_path(self) -> str | None:
        go_mod = os.path.join(self.project_root, "go.mod")
        if not os.path.isfile(go_mod):
            return None
        try:
            with open(go_mod, encoding="utf-8", errors="replace") as f:
                for line in f:
                    stripped = line.strip()
                    if stripped.startswith("module "):
                        return stripped[len("module "):].strip()
        except OSError:
            return None
        return None

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a Go import to a representative file in its package.

        Go packages are directories, not single files — unlike every other
        language here, there's no one file "being" the imported package. To
        fit the one-edge-per-import model the rest of the system uses, this
        resolves to one deterministically chosen (alphabetically first) .go
        file in the target package directory, which is enough to make the
        dependency edge exist and point at the right directory; it is not
        claiming that specific file is uniquely significant.
        """
        module = self.module_path
        if not module:
            return None

        imp = import_text.strip().strip('"')
        if imp == module:
            rel_dir = ""
        elif imp.startswith(module + "/"):
            rel_dir = imp[len(module) + 1:]
        else:
            return None  # third-party or standard library import

        candidate_dir = os.path.join(self.project_root, rel_dir) if rel_dir else self.project_root
        if not os.path.isdir(candidate_dir):
            return None

        try:
            go_files = sorted(
                entry
                for entry in os.listdir(candidate_dir)
                if entry.endswith(".go")
                and os.path.isfile(os.path.join(candidate_dir, entry))
            )
        except OSError:
            return None
        if not go_files:
            return None

        return os.path.join(rel_dir, go_files[0]) if rel_dir else go_files[0]

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
