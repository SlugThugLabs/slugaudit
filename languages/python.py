"""Tree-sitter Python extractor — extracts signatures and imports from .py files."""

import os
import re
from typing import Any

from tree_sitter import Language, Parser
import tree_sitter_python as tspython

from .base import BaseExtractor


class PythonExtractor(BaseExtractor):
    """Extractor for Python source files using tree-sitter."""

    FN_DEF = "function_definition"
    CLASS_DEF = "class_definition"
    DECORATED = "decorated_definition"
    IMPORT = "import_statement"
    IMPORT_FROM = "import_from_statement"
    ASSIGNMENT = "assignment"
    COMMENT = "comment"

    @classmethod
    def name(cls) -> str:
        return "python"

    @classmethod
    def source_extensions(cls) -> set[str]:
        return {".py"}

    @property
    def parser(self) -> Any:
        if self._parser is None:
            py_lang = Language(tspython.language())
            p = Parser(py_lang)
            self._parser = p
        return self._parser

    def _handle_signature_node(self, cursor: Any, source_bytes: bytes, source_lines: list[str], signatures: list[Any], file_path: str) -> None:
        """Handle a single node during signature extraction."""
        node = cursor.node
        node_type = node.type

        if node_type == self.FN_DEF or node_type == self.DECORATED:
            if node_type == self.DECORATED:
                for child in node.named_children:
                    if child.type == self.FN_DEF:
                        sig = self._safe_extract(self._extract_fn, child, source_bytes, source_lines)
                        if sig:
                            signatures.append(sig)
                        break
            else:
                sig = self._safe_extract(self._extract_fn, node, source_bytes, source_lines)
                if sig:
                    signatures.append(sig)

        elif node_type == self.CLASS_DEF:
            sig = self._safe_extract(self._extract_class, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

    def _get_name(self, node: Any, source_bytes: bytes) -> str:
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
        return "unnamed"

    def _collect_docstring(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> str:
        """Extract docstring from the first statement in a body."""
        body = None
        for child in node.named_children:
            if child.type == "block":
                body = child
                break
        if body and body.named_children:
            first = body.named_children[0]
            if first.type == "expression_statement":
                expr = first.named_children[0] if first.named_children else None
                if expr and expr.type == "string":
                    return self.collect_node_text(expr, source_bytes)[:200]
        return ""

    def _extract_fn(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Check for async
            is_async = any(child.type == "async" for child in node.children)

            # Get decorators (tree-sitter doesn't track them directly on decorated_definition's child)
            docstring = self._collect_docstring(node, source_bytes, source_lines)

            # Truncate body
            colon_idx = sig_text.find(":")
            if colon_idx >= 0:
                sig_text = sig_text[:colon_idx + 1].strip()

            return {
                "type": "fn",
                "name": name,
                "signature": sig_text[:500],
                "visibility": "",
                "doc_comment": docstring,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": is_async,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_class(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            docstring = self._collect_docstring(node, source_bytes, source_lines)

            colon_idx = sig_text.find(":")
            if colon_idx >= 0:
                sig_text = sig_text[:colon_idx + 1].strip()

            # Get bases
            bases: list[Any] = []
            for child in node.named_children:
                if child.type == "argument_list":
                    for arg in child.named_children:
                        bases.append(self.collect_node_text(arg, source_bytes))

            return {
                "type": "class",
                "name": name,
                "signature": f"class {name}({', '.join(bases)}):" if bases else f"class {name}:",
                "visibility": "",
                "doc_comment": docstring,
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

        if node.type in (self.IMPORT, self.IMPORT_FROM):
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imp_type = self._classify_import(imp_text)
            imports.append({
                "import_text": imp_text,
                "import_type": imp_type,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
            })

    def _classify_import(self, imp_text: str) -> str:
        """Classify a Python import as internal or external."""
        # Extract the module name
        if imp_text.startswith("from "):
            # from foo.bar import baz → module = foo.bar
            parts = imp_text[5:].split(" import ", 1)
            module = parts[0] if parts else ""
        elif imp_text.startswith("import "):
            # import foo.bar
            module = imp_text[7:].split(" as ")[0].strip()
            # Handle import foo, bar
            module = module.split(",")[0].strip()
        else:
            return "external"

        # Relative imports are always internal
        if module.startswith("."):
            return "internal"

        # Known stdlib modules
        return "external"

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a Python import to a file path.

        Handles:
            from foo.bar import baz  → foo/bar.py or foo/bar/__init__.py
            import foo.bar           → foo/bar.py or foo/bar/__init__.py
            from . import foo        → same dir's __init__.py
            from .bar import baz     → ./bar.py or ./bar/__init__.py
        """
        imp = import_text
        src_dir = os.path.dirname(source_file)

        # Parse the import
        if imp.startswith("from "):
            rest = imp[5:]
            if " import " in rest:
                module_part, _ = rest.split(" import ", 1)
            else:
                return None
        elif imp.startswith("import "):
            module_part = imp[7:].split(" as ")[0].strip()
            # Handle multiple imports: import foo, bar → just take first
            module_part = module_part.split(",")[0].strip()
        else:
            return None

        # Handle relative imports
        if module_part.startswith("."):
            dot_count = len(module_part) - len(module_part.lstrip("."))
            module_name = module_part.lstrip(".")

            dir_path = src_dir
            for _ in range(dot_count - 1):
                dir_path = os.path.dirname(dir_path)

            if module_name:
                candidate = os.path.join(dir_path, module_name.replace(".", "/"))
            else:
                # from . import foo → look at __init__.py in same dir
                candidate = os.path.join(dir_path, "__init__")

            return self._try_py_paths(candidate)

        # Handle absolute imports
        candidate = module_part.replace(".", "/")

        # Try project paths
        # absolute import like foo.bar → look for foo/bar.py anywhere
        # Check if it exists in src/ (common for many projects)
        for base in ("", "src"):
            path = os.path.join(base, candidate) if base else candidate
            result = self._try_py_paths(path)
            if result:
                return result

        # Try relative to source file
        result = self._try_py_paths(os.path.join(src_dir, candidate))
        if result:
            return result

        return None

    def _try_py_paths(self, base_path: str) -> str | None:
        """Try common Python file path conventions."""
        candidates = [
            base_path + ".py",
            os.path.join(base_path, "__init__.py"),
        ]
        for candidate in candidates:
            abspath = os.path.join(self.project_root, candidate)
            if os.path.exists(abspath) and os.path.isfile(abspath):
                return candidate
        return None

    # ── Risk pattern extraction ──────────────────────────────────────────

    def extract_risk_patterns(self, file_path: str, source_bytes: bytes) -> list[dict[str, Any]]:
        """Extract risky Python patterns: eval, exec, os.system, shell=True, bare except."""
        text = source_bytes.decode("utf-8", errors="replace")

        # Filter out comment-only lines
        lines = text.split("\n")
        code_lines = [line for line in lines if not line.strip().startswith("#")]
        code_text = "\n".join(code_lines)

        counts: dict[str, int] = {}
        patterns = [
            (r'\beval\s*\(', 'eval'),
            (r'\bexec\s*\(', 'exec'),
            (r'\bos\.system\s*\(', 'os_system'),
            (r'\bsubprocess\.[a-z_]+\s*\([^)]*shell\s*=\s*True', 'subprocess_shell_true'),
            (r'^\s*except\s*:', 'bare_except'),
        ]

        for pattern, name in patterns:
            matches = re.findall(pattern, code_text, re.MULTILINE)
            if matches:
                counts[name] = len(matches)

        return [{"pattern_type": k, "count": v} for k, v in counts.items() if v > 0]


__all__ = ["PythonExtractor"]
