"""Tree-sitter Rust extractor — extracts signatures and imports from .rs files."""

import os

from tree_sitter import Language, Parser
import tree_sitter_rust as tsrust

from .base import BaseExtractor
from typing import Any


class RustExtractor(BaseExtractor):
    """Extractor for Rust source files using tree-sitter."""

    # Node types from tree-sitter-rust grammar
    FN_ITEM = "function_item"
    FN_SIG = "function_signature"
    STRUCT = "struct_item"
    ENUM = "enum_item"
    TRAIT = "trait_item"
    IMPL = "impl_item"
    TYPE_ALIAS = "type_item"
    CONST = "const_item"
    STATIC = "static_item"
    MACRO = "macro_definition"
    USE_DECL = "use_declaration"
    MOD_DECL = "mod_item"
    VISIBILITY = "visibility_modifier"
    COMMENT = "line_comment"
    BLOCK_COMMENT = "block_comment"

    @classmethod
    def name(cls) -> str:
        return "rust"

    @classmethod
    def source_extensions(cls) -> set[str]:
        return {".rs"}

    @property
    def parser(self) -> Any:
        if self._parser is None:
            rust_lang = Language(tsrust.language())
            p = Parser(rust_lang)
            self._parser = p
        return self._parser

    def _handle_signature_node(self, cursor: Any, source_bytes: bytes, source_lines: list[str], signatures: list[Any], file_path: str) -> None:
        """Handle a single node during signature extraction."""
        node = cursor.node
        node_type = node.type

        if node_type == self.FN_ITEM:
            sig = self._safe_extract(self._extract_fn, node, source_bytes, source_lines, file_path)
            if sig:
                signatures.append(sig)

        elif node_type == self.STRUCT:
            sig = self._safe_extract(self._extract_struct_enum, node, source_bytes, source_lines, "struct")
            if sig:
                signatures.append(sig)

        elif node_type == self.ENUM:
            sig = self._safe_extract(self._extract_struct_enum, node, source_bytes, source_lines, "enum")
            if sig:
                signatures.append(sig)

        elif node_type == self.TRAIT:
            sig = self._safe_extract(self._extract_trait, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.IMPL:
            sig = self._safe_extract(self._extract_impl, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.TYPE_ALIAS:
            sig = self._safe_extract(self._extract_type_alias, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.CONST:
            sig = self._safe_extract(self._extract_const, node, source_bytes, source_lines, "const")
            if sig:
                signatures.append(sig)

        elif node_type == self.STATIC:
            sig = self._safe_extract(self._extract_const, node, source_bytes, source_lines, "static")
            if sig:
                signatures.append(sig)

        elif node_type == self.MACRO:
            sig = self._safe_extract(self._extract_macro, node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

    def _get_visibility(self, node: Any, source_bytes: bytes) -> str:
        """Extract pub/pub(crate) from a definition node."""
        for child in node.children:
            if child.type == self.VISIBILITY:
                return self.collect_node_text(child, source_bytes).strip()
        # Check children directly
        for child in node.named_children:
            if child.type == self.VISIBILITY:
                return self.collect_node_text(child, source_bytes).strip()
        return ""

    def _get_name(self, node: Any, source_bytes: bytes) -> str:
        """Extract the name identifier from a definition node."""
        for child in node.named_children:
            if child.type == "identifier" or child.type == "type_identifier":
                return self.collect_node_text(child, source_bytes).strip()
        return "unnamed"

    def _get_generic_params(self, node: Any, source_bytes: bytes) -> str:
        """Extract generic parameters like <T: Display> from a node."""
        for child in node.children:
            if child.type == "generic_parameters":
                return self.collect_node_text(child, source_bytes).strip()
        return ""

    def _extract_fn(self, node: Any, source_bytes: bytes, source_lines: list[str], file_path: str) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_visibility(node, source_bytes)
            generic_params = self._get_generic_params(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Truncate body if present
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"
            semicolon_idx = sig_text.find(";")
            if semicolon_idx >= 0 and brace_idx < 0:
                sig_text = sig_text[:semicolon_idx + 1].strip()

            # Check for async/unsafe
            is_async = False
            is_unsafe = False
            for child in node.children:
                if child.type == "async":
                    is_async = True
                if child.type == "unsafe":
                    is_unsafe = True

            doc_comment = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": "fn",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc_comment,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": is_async,
                "is_unsafe": is_unsafe,
                "generic_params": generic_params,
            }
        except Exception:
            return None

    def _extract_struct_enum(self, node: Any, source_bytes: bytes, source_lines: list[str], kind: str) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_visibility(node, source_bytes)
            generic_params = self._get_generic_params(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"
            paren_idx = sig_text.find("(")
            if paren_idx >= 0 and (brace_idx < 0 or paren_idx < brace_idx):
                # tuple struct
                close_paren = sig_text.find(")")
                if close_paren >= 0:
                    sig_text = sig_text[:close_paren + 1].strip()

            doc_comment = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": kind,
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc_comment,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": generic_params,
            }
        except Exception:
            return None

    def _extract_trait(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_visibility(node, source_bytes)
            generic_params = self._get_generic_params(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            doc_comment = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": "trait",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc_comment,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": generic_params,
            }
        except Exception:
            return None

    def _extract_impl(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            sig_text = self.collect_node_text(node, source_bytes)
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            # Extract the type being implemented
            # Look for the type_identifier after 'impl'
            impl_for = ""
            for child in node.named_children:
                if child.type in ("type_identifier", "generic_type", "qualified_type"):
                    impl_for = self.collect_node_text(child, source_bytes)
                    break

            doc_comment = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": "impl",
                "name": impl_for or "unknown",
                "signature": sig_text[:500],
                "visibility": "",
                "doc_comment": doc_comment,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_type_alias(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_visibility(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            doc_comment = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": "type_alias",
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc_comment,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_const(self, node: Any, source_bytes: bytes, source_lines: list[str], kind: str) -> dict[str, Any] | None:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_visibility(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            doc_comment = self._get_doc_comment_above(node, source_bytes, source_lines)

            return {
                "type": kind,
                "name": name,
                "signature": sig_text[:500],
                "visibility": visibility,
                "doc_comment": doc_comment,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
                "is_async": False,
                "is_unsafe": False,
                "generic_params": "",
            }
        except Exception:
            return None

    def _extract_macro(self, node: Any, source_bytes: bytes, source_lines: list[str]) -> dict[str, Any] | None:
        try:
            name = ""
            for child in node.named_children:
                if child.type == "identifier":
                    name = self.collect_node_text(child, source_bytes)
                    break
            if not name:
                name = "unnamed_macro"

            sig_text = self.collect_node_text(node, source_bytes)[:200]

            return {
                "type": "macro",
                "name": name,
                "signature": sig_text,
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

    # ── Risk pattern extraction ──────────────────────────────────────────

    def extract_risk_patterns(self, file_path: str, source_bytes: bytes) -> list[dict[str, Any]]:
        """Extract risky Rust patterns: unwrap, expect, unsafe blocks, panic, as casts."""
        parser = self.get_parser()
        tree = parser.parse(source_bytes)

        counts: dict[str, int] = {}
        self._walk_risk(tree.root_node, source_bytes, counts)

        return [{"pattern_type": k, "count": v} for k, v in counts.items() if v > 0]

    def _walk_risk(self, node: Any, source_bytes: bytes, counts: dict[str, int]) -> None:
        t = node.type

        if t == "unsafe_block":
            counts["unsafe_blocks"] = counts.get("unsafe_blocks", 0) + 1

        elif t == "as_expression":
            counts["as_casts"] = counts.get("as_casts", 0) + 1

        elif t == "macro_invocation":
            text = self.collect_node_text(node, source_bytes)
            if text.startswith("panic!"):
                counts["panic"] = counts.get("panic", 0) + 1
            elif text.startswith("unreachable!"):
                counts["unreachable"] = counts.get("unreachable", 0) + 1

        elif t == "call_expression":
            method = self._get_call_method_name(node, source_bytes)
            if method in ("unwrap", "expect"):
                counts[method] = counts.get(method, 0) + 1

        for child in node.children:
            self._walk_risk(child, source_bytes, counts)

    def _get_call_method_name(self, node: Any, source_bytes: bytes) -> str | None:
        """Get the method name if this call_expression is a method call."""
        for child in node.children:
            if child.type == "field_expression":
                for grandchild in child.named_children:
                    if grandchild.type == "field_identifier":
                        return self.collect_node_text(grandchild, source_bytes)
        return None

    # ── Import extraction ──────────────────────────────────────────────────

    def _handle_import_node(self, cursor: Any, source_bytes: bytes, imports: list[Any], file_path: str) -> None:
        """Handle a single node during import extraction."""
        node = cursor.node

        if node.type == self.USE_DECL:
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imp_type = self._classify_import(imp_text)
            imports.append({
                "import_text": imp_text,
                "import_type": imp_type,
                "line_start": node.start_point[0] + 1,
                "line_end": node.end_point[0] + 1,
            })

        elif node.type == self.MOD_DECL:
            # pub mod foo; — not an import but defines a module relationship
            if node.children and node.children[-1].type == ";":
                imp_text = self.collect_node_text(node, source_bytes).strip()
                imports.append({
                    "import_text": imp_text,
                    "import_type": "internal",
                    "line_start": node.start_point[0] + 1,
                    "line_end": node.end_point[0] + 1,
                })

    def _classify_import(self, imp_text: str) -> str:
        """Classify an import as internal or external."""
        # Strip 'use ' prefix and trailing ';' for prefix matching
        imp = imp_text
        if imp.startswith("use "):
            imp = imp[4:]
        if imp.endswith(";"):
            imp = imp[:-1]

        # External crates start with the crate name directly (not crate::, super::, self::)
        internal_prefixes = ("crate::", "super::", "self::")
        if any(imp.startswith(p) for p in internal_prefixes):
            return "internal"

        # Known external crates (Rust standard library)
        external_prefixes = (
            "std::", "core::", "alloc::", "proc_macro::",
            "test::", "bench::", "compiler_builtins::",
        )
        if any(imp.startswith(p) for p in external_prefixes):
            return "external"

        # If it starts with a path pattern that looks like a workspace crate
        # (contains ::), treat as internal for dependency resolution
        return "internal"

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a Rust use statement to a file path.

        Handles:
            use crate::foo::bar        → src/foo/bar.rs or src/foo/bar/mod.rs
            use super::baz             → parent module's baz
            use slugid_infrastructure::x → workspace crate (stays external for now)
        """
        imp = import_text

        # Strip leading 'use ' and trailing ';'
        if imp.startswith("use "):
            imp = imp[4:]
        if imp.endswith(";"):
            imp = imp[:-1]

        # Strip pub mod to get module name
        if imp.startswith("pub mod "):
            mod_name = imp[8:].strip()
            return self._resolve_mod_in_same_dir(mod_name, source_file)

        # Handle crate:: prefix
        if imp.startswith("crate::"):
            path_part = imp[7:]
            # Split on :: and reconstruct as file path
            segments = path_part.split("::")
            return self._path_segments_to_file(segments, source_file)

        # Handle super:: prefix (walk up the directory tree)
        if imp.startswith("super::"):
            # Count super:: prefixes
            parts = imp.split("::")
            super_count = 0
            for p in parts:
                if p == "super":
                    super_count += 1
                else:
                    break
            remaining = "::".join(parts[super_count:])

            # Walk up from source file's directory
            src_dir = os.path.dirname(source_file)
            for _ in range(super_count):
                src_dir = os.path.dirname(src_dir)

            segments = remaining.split("::")
            candidate = os.path.join(src_dir, *segments)
            return self._try_file_paths(candidate)

        # Handle self:: prefix
        if imp.startswith("self::"):
            segments = imp[6:].split("::")
            src_dir = os.path.dirname(source_file)
            candidate = os.path.join(src_dir, *segments)
            return self._try_file_paths(candidate)

        # Handle just a path (no prefix) — probably a workspace crate
        # Don't resolve these for now
        return None

    def _resolve_mod_in_same_dir(self, mod_name: str, source_file: str) -> str | None:
        """Resolve a module name in the same directory as source_file."""
        src_dir = os.path.dirname(source_file)
        candidate = os.path.join(src_dir, mod_name)
        return self._try_file_paths(candidate)

    def _path_segments_to_file(self, segments: list[str], source_file: str) -> str | None:
        """Convert path segments (e.g. ['entities', 'card']) to a file path."""
        # Remove last segment if it refers to a specific item (not a module)
        # Rust convention: use crate::entities::card → src/entities/card.rs
        if len(segments) >= 1:
            candidate = os.path.join("src", *segments)
            return self._try_file_paths(candidate)
        return None

    def _try_file_paths(self, base_path: str) -> str | None:
        """Try common Rust file path conventions."""
        candidates = [
            base_path + ".rs",
            base_path + "/mod.rs",
            base_path.rsplit("/", 1)[0] + ".rs" if "/" in base_path else base_path + ".rs",
        ]
        for candidate in candidates:
            abspath = os.path.join(self.project_root, candidate)
            if os.path.exists(abspath) and os.path.isfile(abspath):
                return candidate
        return None


__all__ = ["RustExtractor"]
