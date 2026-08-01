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
        """Handle a single node during signature extraction.

        `decorated_definition` itself needs no special case: the walker
        already visits its wrapped function_definition/class_definition as
        an ordinary named child, which is exactly the plain dispatch below.
        A `decorated_definition` branch that reached in and extracted that
        child *again* used to double-count every decorated function (the
        same wrapped node got extracted once here and once more when the
        walker reached it naturally) — decorated classes never had this bug
        because CLASS_DEF was only ever handled by the plain branch.
        """
        node = cursor.node
        node_type = node.type

        if node_type == self.FN_DEF:
            sig = self._safe_extract(self._extract_fn, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.CLASS_DEF:
            sig = self._safe_extract(self._extract_class, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.ASSIGNMENT:
            var_sigs = self._safe_extract(self._extract_assignment, node, source_bytes)
            if var_sigs:
                signatures.extend(var_sigs)

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

    def _assignment_target_names(self, target: Any) -> list[Any]:
        """Every name node an assignment target actually binds.

        `identifier` (`x = ...`) and `attribute` (`self.x = ...`, using its
        trailing identifier) are terminal cases. `pattern_list`/`tuple_pattern`
        (`a, b = ...`, `self.y, self.z = ...`) recurse into each element so
        nested attribute targets are still found. Anything else (subscript
        targets like `d[k] = v`, starred targets) isn't a variable binding
        and is deliberately left unhandled.
        """
        if target.type == "identifier":
            return [target]
        if target.type == "attribute":
            attr_children = target.named_children
            return [attr_children[-1]] if attr_children and attr_children[-1].type == "identifier" else []
        if target.type in ("pattern_list", "tuple_pattern"):
            names = []
            for child in target.named_children:
                names.extend(self._assignment_target_names(child))
            return names
        return []

    def _extract_assignment(self, node: Any, source_bytes: bytes) -> list[dict[str, Any]]:
        """Every name bound by one assignment statement, at any depth.

        Only the first named child (the target, always positionally first in
        this grammar) is inspected — a type annotation (`y: int = 2`) puts an
        `identifier` inside its `type` node too, which must never be mistaken
        for a second variable name.
        """
        if not node.named_children:
            return []
        names = self._assignment_target_names(node.named_children[0])
        if not names:
            return []

        sig_text = self.collect_node_text(node, source_bytes).strip()[:200]
        line_start = node.start_point[0] + 1
        line_end = node.end_point[0] + 1
        return [
            {
                "type": "variable",
                "name": self.collect_node_text(name_node, source_bytes).strip(),
                "signature": sig_text,
                "visibility": "",
                "doc_comment": "",
                "line_start": line_start,
                "line_end": line_end,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
            for name_node in names
        ]

    # ── Import extraction ──────────────────────────────────────────────────

    def _handle_import_node(self, cursor: Any, source_bytes: bytes, imports: list[Any], file_path: str) -> None:
        """Handle a single node during import extraction."""
        node = cursor.node

        if node.type in (self.IMPORT, self.IMPORT_FROM):
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imp_type = self._classify_import(imp_text, file_path)
            imports.append({
                "import_text": imp_text,
                "import_type": imp_type,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
            })

    def _classify_import(self, imp_text: str, file_path: str) -> str:
        """Classify a Python import as internal or external.

        Relative imports (`from . import x`) are unambiguously internal by
        syntax. Everything else genuinely can't be classified from syntax
        alone — `from foo import bar` is indistinguishable from a top-level
        local module `foo.py` or the third-party package `foo` without
        knowing what's actually in the project. Attempt the same resolution
        resolve_import would do and classify by whether it actually finds a
        local file, mirroring how RustExtractor's crate_map-based
        classification stays consistent with its own resolution by
        construction rather than guessing separately.
        """
        module = self._extract_module_part(imp_text)
        if module is None:
            return "external"

        if module.startswith("."):
            return "internal"

        if self.resolve_import(imp_text, file_path, {}) is not None:
            return "internal"
        return "external"

    def _extract_module_part(self, imp_text: str) -> str | None:
        """Pull the dotted module path out of an import statement's text."""
        if imp_text.startswith("from "):
            rest = imp_text[5:]
            if " import " not in rest:
                return None
            return rest.split(" import ", 1)[0].strip()
        if imp_text.startswith("import "):
            module = imp_text[7:].split(" as ")[0].strip()
            return module.split(",")[0].strip()
        return None

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a Python import to a file path.

        Handles:
            from foo.bar import baz  → foo/bar.py or foo/bar/__init__.py
            import foo.bar           → foo/bar.py or foo/bar/__init__.py
            from . import foo        → same dir's __init__.py
            from .bar import baz     → ./bar.py or ./bar/__init__.py
        """
        src_dir = os.path.dirname(source_file)

        module_part = self._extract_module_part(import_text)
        if module_part is None:
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
