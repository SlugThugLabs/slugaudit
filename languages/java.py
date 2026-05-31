"""Tree-sitter Java extractor — extracts signatures and imports from .java files."""

import os
from typing import Optional

from tree_sitter import Language, Parser
import tree_sitter_java as tsjava

from .base import BaseExtractor


class JavaExtractor(BaseExtractor):
    """Extractor for Java source files using tree-sitter."""

    CLASS_DECL = "class_declaration"
    INTERFACE_DECL = "interface_declaration"
    ENUM_DECL = "enum_declaration"
    RECORD_DECL = "record_declaration"
    METHOD_DECL = "method_declaration"
    CONSTRUCTOR_DECL = "constructor_declaration"
    IMPORT_DECL = "import_declaration"
    MODIFIERS = "modifiers"
    COMMENT = "line_comment"
    BLOCK_COMMENT = "block_comment"

    @classmethod
    def name(cls) -> str:
        return "java"

    @classmethod
    def source_extensions(cls) -> set:
        return {".java"}

    @property
    def parser(self):
        if self._parser is None:
            java_lang = Language(tsjava.language())
            p = Parser(java_lang)
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

        if node_type == self.CLASS_DECL:
            sig = self._extract_type(node, source_bytes, source_lines, "class")
            if sig:
                signatures.append(sig)

        elif node_type == self.INTERFACE_DECL:
            sig = self._extract_type(node, source_bytes, source_lines, "interface")
            if sig:
                signatures.append(sig)

        elif node_type == self.ENUM_DECL:
            sig = self._extract_type(node, source_bytes, source_lines, "enum")
            if sig:
                signatures.append(sig)

        elif node_type == self.RECORD_DECL:
            sig = self._extract_type(node, source_bytes, source_lines, "record")
            if sig:
                signatures.append(sig)

        elif node_type == self.METHOD_DECL:
            sig = self._extract_method(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        elif node_type == self.CONSTRUCTOR_DECL:
            sig = self._extract_method(node, source_bytes, source_lines)
            if sig:
                signatures.append(sig)

        # Recurse into children
        if cursor.goto_first_child():
            self._walk_tree(cursor, source_bytes, source_lines, signatures)
            while cursor.goto_next_sibling():
                self._walk_tree(cursor, source_bytes, source_lines, signatures)
            cursor.goto_parent()

    def _get_modifiers(self, node, source_bytes) -> str:
        """Extract visibility/access modifiers from a definition node."""
        mods = []
        for child in node.named_children:
            if child.type == self.MODIFIERS:
                for mod in child.named_children:
                    mods.append(self.collect_node_text(mod, source_bytes).strip())
        return " ".join(mods)

    def _get_name(self, node, source_bytes) -> str:
        for child in node.named_children:
            if child.type == "identifier":
                return self.collect_node_text(child, source_bytes).strip()
        return "unnamed"

    def _extract_method(self, node, source_bytes, source_lines) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_modifiers(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Truncate body
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

            kind = "constructor" if node.type == self.CONSTRUCTOR_DECL else "method"
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

    def _extract_type(self, node, source_bytes, source_lines, kind: str) -> Optional[dict]:
        try:
            name = self._get_name(node, source_bytes)
            visibility = self._get_modifiers(node, source_bytes)
            sig_text = self.collect_node_text(node, source_bytes)

            # Truncate body
            brace_idx = sig_text.find("{")
            if brace_idx >= 0:
                sig_text = sig_text[:brace_idx].strip() + " { ... }"

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

        if node.type == self.IMPORT_DECL:
            imp_text = self.collect_node_text(node, source_bytes).strip()
            imp_type = self._classify_import(imp_text)
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

    def _classify_import(self, imp_text: str) -> str:
        """Classify a Java import as internal or external."""
        imp = imp_text.replace("import ", "").replace(";", "").strip()
        if imp.startswith("static "):
            imp = imp[7:].strip()

        # Java standard library packages
        std_pkgs = (
            "java.", "javax.", "javafx.", "com.sun.",
            "org.w3c.", "org.xml.", "org.ietf.jgss",
            "org.omg.", "org.omg.CORBA",
        )
        if any(imp.startswith(p) for p in std_pkgs):
            return "external"

        # If it contains the project name or matches local paths, it's internal
        # Default to external for wide Java ecosystem
        return "external"

    # ── Import resolution ──────────────────────────────────────────────────

    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict) -> Optional[str]:
        """Resolve a Java import to a file path.
        
        import com.example.project.util.StringUtils
          → com/example/project/util/StringUtils.java
        """
        imp = import_text.replace("import ", "").replace(";", "").strip()
        if imp.startswith("static "):
            imp = imp[7:].strip()

        # Convert package path to file path
        path = imp.replace(".", "/") + ".java"

        # Check if this file exists in the project
        abspath = os.path.join(self.project_root, path)
        if os.path.exists(abspath) and os.path.isfile(abspath):
            return path

        # Check in src/ directory (standard Maven/Gradle layout)
        src_path = os.path.join("src", path)
        abspath = os.path.join(self.project_root, src_path)
        if os.path.exists(abspath) and os.path.isfile(abspath):
            return src_path

        # Check for main/java/ source root
        mvn_path = os.path.join("src/main/java", path)
        abspath = os.path.join(self.project_root, mvn_path)
        if os.path.exists(abspath) and os.path.isfile(abspath):
            return mvn_path

        return None


__all__ = ["JavaExtractor"]
