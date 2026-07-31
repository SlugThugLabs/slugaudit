"""Tree-sitter Rust extractor — extracts signatures and imports from .rs files."""

import os
import posixpath
import tomllib

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

    # Cargo manifest directories to never descend into while mapping crates.
    _CARGO_SKIP_DIRS = frozenset({"target", ".git", "node_modules", ".venv", "venv"})

    def __init__(self, project_root: str) -> None:
        super().__init__(project_root)
        # Normalized crate (import) name -> crate source root, project-root-relative.
        # Built lazily and cached: one filesystem walk per extractor instance,
        # i.e. once per sync, never once per import.
        self._crate_map: dict[str, str] | None = None
        # hub file path -> its extracted `use` declarations, cached per sync
        # (see _hub_reexports / _follow_reexport).
        self._reexport_cache: dict[str, list[dict[str, Any]]] = {}

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
        """Iteratively visit every named node (order doesn't matter for counting).

        Explicit stack rather than recursion — see BaseExtractor._walk_tree
        for why: a pathologically deep expression tree must not raise
        RecursionError mid-sync. Named children only, matching the same
        walker's anonymous-token rationale, even though none of the four
        matched types here are known to collide with a keyword token today.
        """
        stack: list[Any] = [node]
        while stack:
            current = stack.pop()
            t = current.type

            if t == "unsafe_block":
                counts["unsafe_blocks"] = counts.get("unsafe_blocks", 0) + 1

            elif t == "as_expression":
                counts["as_casts"] = counts.get("as_casts", 0) + 1

            elif t == "macro_invocation":
                text = self.collect_node_text(current, source_bytes)
                if text.startswith("panic!"):
                    counts["panic"] = counts.get("panic", 0) + 1
                elif text.startswith("unreachable!"):
                    counts["unreachable"] = counts.get("unreachable", 0) + 1

            elif t == "call_expression":
                method = self._get_call_method_name(current, source_bytes)
                if method in ("unwrap", "expect"):
                    counts[method] = counts.get(method, 0) + 1

            stack.extend(current.named_children)

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
            # named_children is [visibility_modifier?, path_node]; the path
            # node is always last regardless of whether visibility is present.
            named = node.named_children
            path_node = named[-1] if named else None
            expanded = self._expand_use_node(path_node, source_bytes) if path_node is not None else []
            if not expanded:
                # Defensive fallback for a grammar shape we didn't anticipate —
                # keep the raw text rather than silently dropping the import.
                raw = self.collect_node_text(node, source_bytes).strip()
                expanded = [raw[4:].rstrip(";").strip()] if raw.startswith("use ") else [raw]

            for path in expanded:
                imp_text = f"use {path};"
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

    def _expand_use_node(self, node: Any, source_bytes: bytes) -> list[str]:
        """Expand a `use` path node into fully-qualified leaf import paths.

        Handles arbitrary nesting of brace groups (`a::{b::{c, d}, e}`),
        `self` inside a group (`a::{self, b}` -> `a`, `a::b`), glob imports
        (`a::*` -> `a`), and `as` aliases (resolved by the real path, alias
        discarded). Leaf paths never contain braces or wildcards.
        """
        node_type = node.type

        if node_type == "use_list":
            expanded: list[str] = []
            for child in node.named_children:
                expanded.extend(self._expand_use_node(child, source_bytes))
            return expanded

        if node_type == "scoped_use_list":
            named = node.named_children
            if len(named) < 2:
                return []
            prefix_text = self.collect_node_text(named[0], source_bytes).strip()
            suffixes = self._expand_use_node(named[-1], source_bytes)
            results: list[str] = []
            for suffix in suffixes:
                if suffix in ("", "self"):
                    results.append(prefix_text)
                else:
                    results.append(f"{prefix_text}::{suffix}")
            return results

        if node_type == "use_as_clause":
            # First named child is the real path; the alias identifier is
            # irrelevant to resolution and classification.
            named = node.named_children
            if not named:
                return []
            return self._expand_use_node(named[0], source_bytes)

        if node_type == "use_wildcard":
            text = self.collect_node_text(node, source_bytes).strip()
            if text.endswith("::*"):
                return [text[:-3]]
            if text == "*":
                return []
            return [text[:-1].rstrip(":")] if text.endswith("*") else [text]

        if node_type in ("identifier", "type_identifier", "crate", "self", "super", "scoped_identifier"):
            # Leaf or already-flat dotted path (may itself embed crate/self/super
            # at its root) — the raw source slice is exactly the path text.
            text = self.collect_node_text(node, source_bytes).strip()
            return [text] if text else []

        # Unrecognized node type: fall back to raw text rather than dropping it.
        text = self.collect_node_text(node, source_bytes).strip()
        return [text] if text else []

    def _classify_import(self, imp_text: str) -> str:
        """Classify an import as internal (workspace crate) or external."""
        # Strip 'use ' prefix and trailing ';' for prefix matching
        imp = imp_text
        if imp.startswith("use "):
            imp = imp[4:]
        if imp.endswith(";"):
            imp = imp[:-1]
        imp = imp.strip()

        # References into the current crate are always internal.
        internal_prefixes = ("crate::", "super::", "self::")
        if any(imp.startswith(p) for p in internal_prefixes) or imp in ("crate", "super", "self"):
            return "internal"

        # Rust standard library / toolchain-provided — always external.
        external_prefixes = (
            "std::", "core::", "alloc::", "proc_macro::",
            "test::", "bench::", "compiler_builtins::",
        )
        if any(imp.startswith(p) for p in external_prefixes) or imp in ("std", "core", "alloc"):
            return "external"

        # Otherwise: internal only if the first segment names a crate that
        # actually lives in this workspace. Everything else (egui, serde,
        # tracing, wgpu, ...) is a third-party dependency.
        first_seg = imp.split("::", 1)[0].strip()
        if first_seg in self.crate_map:
            return "internal"
        return "external"

    # ── Workspace crate map ─────────────────────────────────────────────────

    @property
    def crate_map(self) -> dict[str, str]:
        """Normalized crate (import) name -> crate source root (project-root-relative).

        Built once per extractor instance (one instance is created per sync
        in ImportService._extractors) and cached — never re-parsed per import.
        """
        if self._crate_map is None:
            self._crate_map = self._build_crate_map()
        return self._crate_map

    def _find_cargo_tomls(self) -> list[str]:
        """Absolute paths of every Cargo.toml under project_root, skipping build/vcs dirs."""
        found: list[str] = []
        for dirpath, dirnames, filenames in os.walk(self.project_root):
            dirnames[:] = [
                d for d in dirnames
                if d not in self._CARGO_SKIP_DIRS and not d.startswith(".")
            ]
            if "Cargo.toml" in filenames:
                found.append(os.path.join(dirpath, "Cargo.toml"))
        return found

    def _join_rel(self, root: str, *parts: str) -> str:
        """Join a project-root-relative directory with path parts (posix-style).

        `root` of "" or "." means the project root itself.
        """
        segments = [root.strip("/")] if root and root != "." else []
        segments.extend(p for p in parts if p)
        return "/".join(segments)

    def _build_crate_map(self) -> dict[str, str]:
        """Discover every crate in the workspace and its source root.

        There is no [workspace] members list to trust here — crates are
        declared as path dependencies from the root package. So: glob every
        Cargo.toml, read [package] name (normalizing '-' to '_' to match how
        Rust import paths spell the crate), and honor [lib] path if present;
        otherwise the source root is '<crate_dir>/src'.
        """
        crate_map: dict[str, str] = {}
        for cargo_path in self._find_cargo_tomls():
            try:
                with open(cargo_path, "rb") as fh:
                    data = tomllib.load(fh)
            except (OSError, tomllib.TOMLDecodeError):
                continue

            package = data.get("package")
            if not isinstance(package, dict):
                continue
            name = package.get("name")
            if not isinstance(name, str) or not name:
                continue
            normalized_name = name.replace("-", "_")

            crate_dir_abs = os.path.dirname(cargo_path)
            crate_dir_rel = os.path.relpath(crate_dir_abs, self.project_root)
            crate_dir_rel = "" if crate_dir_rel == "." else crate_dir_rel.replace(os.sep, "/")

            lib_section = data.get("lib")
            lib_path = lib_section.get("path") if isinstance(lib_section, dict) else None
            if isinstance(lib_path, str) and lib_path:
                lib_dir = posixpath.dirname(lib_path.replace(os.sep, "/"))
                src_root = self._join_rel(crate_dir_rel, lib_dir) if lib_dir else crate_dir_rel
            else:
                src_root = self._join_rel(crate_dir_rel, "src")

            crate_map[normalized_name] = src_root
        return crate_map

    def _owning_crate_source_root(self, source_file: str) -> str:
        """Source root of the crate whose source tree contains `source_file`.

        `crate::` must resolve against the root of the crate that *owns* the
        file doing the importing, not against project_root/src. Chosen by
        longest matching source-root prefix (the workspace root crate's
        source root, e.g. "src", always matches everything and so only wins
        when nothing more specific does).
        """
        best_root = ""
        best_len = -1
        for root in self.crate_map.values():
            if root == "":
                matches, length = True, 0
            else:
                matches = source_file == root or source_file.startswith(root + "/")
                length = len(root)
            if matches and length > best_len:
                best_root = root
                best_len = length
        return best_root

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve a Rust use statement to a file path.

        Handles:
            use crate::foo::bar          -> resolved against the owning crate's source root
            use super::baz               -> parent module's baz
            use slugid_infrastructure::x -> workspace crate's source root (cross-crate)
            use serde::Deserialize       -> external crate, unresolvable -> None

        Only ever returns a path that is a key of `path_to_id`, so a resolved
        edge can never point at a file absent from the index.
        """
        imp = import_text

        # Strip leading 'use ' and trailing ';'
        if imp.startswith("use "):
            imp = imp[4:]
        if imp.endswith(";"):
            imp = imp[:-1]
        imp = imp.strip()

        # Strip pub mod to get module name
        if imp.startswith("pub mod "):
            mod_name = imp[8:].strip()
            return self._resolve_mod_in_same_dir(mod_name, source_file, path_to_id)

        # Handle crate:: prefix — resolve against the *owning* crate's root,
        # not project_root/src.
        if imp.startswith("crate::") or imp == "crate":
            path_part = imp[7:] if imp.startswith("crate::") else ""
            segments = [s for s in path_part.split("::") if s]
            crate_root = self._owning_crate_source_root(source_file)
            return self._segments_to_file(segments, crate_root, path_to_id)

        # Handle super:: prefix (walk up the directory tree)
        if imp.startswith("super::") or imp == "super":
            parts = imp.split("::")
            super_count = 0
            for p in parts:
                if p == "super":
                    super_count += 1
                else:
                    break
            remaining = [p for p in parts[super_count:] if p]

            src_dir = posixpath.dirname(source_file)
            for _ in range(super_count):
                src_dir = posixpath.dirname(src_dir)

            return self._segments_to_file(remaining, src_dir, path_to_id)

        # Handle self:: prefix
        if imp.startswith("self::") or imp == "self":
            path_part = imp[6:] if imp.startswith("self::") else ""
            segments = [s for s in path_part.split("::") if s]
            src_dir = posixpath.dirname(source_file)
            return self._segments_to_file(segments, src_dir, path_to_id)

        # Cross-crate: first segment names a workspace crate.
        first_seg, sep, rest = imp.partition("::")
        first_seg = first_seg.strip()
        if first_seg in self.crate_map:
            remaining = [s for s in rest.split("::") if s] if sep else []
            crate_root = self.crate_map[first_seg]
            return self._segments_to_file(remaining, crate_root, path_to_id)

        # First segment is std/core/alloc or an unknown (third-party) crate.
        return None

    def _resolve_mod_in_same_dir(
        self, mod_name: str, source_file: str, path_to_id: dict[str, Any]
    ) -> str | None:
        """Resolve a module name in the same directory as source_file."""
        src_dir = posixpath.dirname(source_file)
        return self._segments_to_file([mod_name], src_dir, path_to_id)

    # A module re-exporting an item can itself re-export from another module
    # that re-exports it again; cap the chase rather than risk a cycle.
    _MAX_REEXPORT_DEPTH = 4

    def _segments_to_file(
        self, segments: list[str], root: str, path_to_id: dict[str, Any], _depth: int = 0
    ) -> str | None:
        """Convert path segments relative to `root` into an indexed file path."""
        if not segments:
            # The module/crate root itself (e.g. `use crate;` or a resolved
            # crate name with no further path).
            return self._resolve_module_root(root, path_to_id)

        base_path = self._join_rel(root, *segments)
        resolved = self._try_file_paths(base_path, path_to_id)
        if resolved is not None:
            return resolved

        # The last segment may name an item (struct/fn/trait/const/re-export)
        # rather than a module — fall back to its containing module: either
        # the parent directory's own module file, or, when the item sits
        # directly at the root of this crate/module (e.g.
        # `use contracts::ApplicationService;`), the module root itself.
        parent_root = self._join_rel(root, *segments[:-1]) if len(segments) > 1 else root
        hub = self._resolve_module_root(parent_root, path_to_id)
        if hub is None:
            return None
        if _depth >= self._MAX_REEXPORT_DEPTH:
            return hub

        # The item may not be defined in `hub` itself but re-exported from a
        # sibling module (`pub use sibling::{..., Item, ...};`), which is the
        # common `pub use ports::{ApplicationService, ...}` pattern at a
        # crate root. Follow that chain to the real definition site so the
        # dependency edge lands on the file that actually defines the item.
        item_name = segments[-1]
        followed = self._follow_reexport(hub, parent_root, item_name, path_to_id, _depth + 1)
        return followed if followed is not None else hub

    def _resolve_module_root(self, root: str, path_to_id: dict[str, Any]) -> str | None:
        """Resolve `root` itself (a directory) to its module/crate entry file."""
        for candidate_name in ("mod.rs", "lib.rs", "main.rs"):
            candidate = self._join_rel(root, candidate_name)
            if candidate in path_to_id:
                return candidate
        # `root` may itself be a plain file module rather than a directory
        # module, e.g. root="domain/src/entities" -> "domain/src/entities.rs".
        file_candidate = root + ".rs"
        if file_candidate in path_to_id:
            return file_candidate
        return None

    def _try_file_paths(self, base_path: str, path_to_id: dict[str, Any]) -> str | None:
        """Try common Rust file path conventions against the indexed file map.

        Resolves only against `path_to_id` (never the filesystem) so a
        resolved_path can never reference a file absent from the index.
        """
        candidates = [base_path + ".rs", base_path + "/mod.rs"]
        for candidate in candidates:
            if candidate in path_to_id:
                return candidate
        return None

    def _hub_reexports(self, hub_path: str) -> list[dict[str, Any]]:
        """`use` declarations found in `hub_path`, cached for this sync.

        Reads the real file (needed to see the re-export's target module —
        `path_to_id` alone can't tell us that); the return value from
        `resolve_import` is still always gated through `path_to_id`, so this
        can never manufacture a resolved_path absent from the index.
        """
        cached = self._reexport_cache.get(hub_path)
        if cached is not None:
            return cached
        abs_path = os.path.join(self.project_root, hub_path)
        try:
            with open(abs_path, "rb") as fh:
                content = fh.read()
        except OSError:
            self._reexport_cache[hub_path] = []
            return []
        imports = self.extract_imports(hub_path, content)
        self._reexport_cache[hub_path] = imports
        return imports

    def _follow_reexport(
        self,
        hub_path: str,
        hub_root: str,
        item_name: str,
        path_to_id: dict[str, Any],
        depth: int,
    ) -> str | None:
        """If `hub_path` re-exports `item_name` from a sibling module, follow it."""
        for reexport in self._hub_reexports(hub_path):
            text = reexport["import_text"]
            if not (text.startswith("use ") and text.endswith(";")):
                continue
            parts = [p for p in text[4:-1].split("::") if p]
            if len(parts) < 2 or parts[-1] != item_name:
                continue
            prefix = parts[:-1]
            if prefix and prefix[0] in ("crate", "self", "super"):
                prefix = prefix[1:]
            if not prefix:
                continue
            candidate = self._segments_to_file(prefix + [item_name], hub_root, path_to_id, depth)
            if candidate is not None and candidate != hub_path:
                return candidate
        return None


__all__ = ["RustExtractor"]
