"""Base extractor class for language-specific signature and import extraction."""

import os
import hashlib
from abc import ABC, abstractmethod
from datetime import datetime, timezone
from typing import Optional


class BaseExtractor(ABC):
    """Abstract base for language-specific code extractors.
    
    Subclasses implement tree-sitter based extraction of:
    - Function/method/class/struct/enum/trait signatures
    - Import/use/include statements
    - Visibility modifiers
    - Doc comments
    """

    @classmethod
    @abstractmethod
    def name(cls) -> str:
        """Language name, e.g. 'rust', 'python'."""
        ...

    @classmethod
    @abstractmethod
    def source_extensions(cls) -> set:
        """Set of file extensions for this language."""
        ...

    def __init__(self, project_root: str):
        self.project_root = os.path.abspath(project_root)
        self._parser = None

    @property
    @abstractmethod
    def parser(self):
        """Lazy-initialized tree-sitter parser."""
        ...

    @abstractmethod
    def extract_signatures(self, file_path: str, source_bytes: bytes) -> list[dict]:
        """Extract signatures from source bytes.
        
        Returns list of dicts with:
            type: str (fn, struct, class, enum, trait, impl, const, type_alias, macro)
            name: str
            signature: str (truncated signature text)
            visibility: str ('pub', 'export', '', etc.)
            doc_comment: str
            line_start: int (1-based)
            line_end: int (1-based)
            is_async: bool
            is_unsafe: bool (Rust only)
            generic_params: str
        """
        ...

    @abstractmethod
    def extract_imports(self, file_path: str, source_bytes: bytes) -> list[dict]:
        """Extract import/use/include statements from source bytes.
        
        Returns list of dicts with:
            import_text: str (raw import statement text)
            import_type: str ('internal' or 'external')
            line_start: int (1-based)
            line_end: int (1-based)
        """
        ...

    @abstractmethod
    def resolve_import(self, import_text: str, source_file: str, path_to_id: dict) -> Optional[str]:
        """Resolve an import statement to a file path relative to project root.
        
        Args:
            import_text: The raw import statement
            source_file: The path of the file containing the import
            path_to_id: Mapping of file paths to file IDs
        
        Returns:
            Resolved file path relative to project root, or None if external/unresolvable
        """
        ...

    def get_parser(self):
        """Get or create the tree-sitter parser."""
        if self._parser is None:
            self._parser = self.parser
        return self._parser

    def hash_file(self, filepath: str) -> str:
        """SHA-256 hash of file contents."""
        h = hashlib.sha256()
        with open(filepath, "rb") as f:
            h.update(f.read())
        return h.hexdigest()

    def find_source_files(self) -> list[str]:
        """Find all source files in the project matching this language's extensions.
        
        Returns paths relative to project_root.
        Skips .git, target, node_modules, and hidden directories.
        """
        skip_dirs = {".git", "target", "node_modules", "__pycache__",
                      ".venv", "venv", ".planning", ".claude",
                      "dist", "build", ".next", ".nuxt"}
        exts = self.source_extensions()
        files = []
        for dirpath, dirnames, filenames in os.walk(self.project_root):
            dirnames[:] = [d for d in dirnames
                           if not d.startswith(".") and d not in skip_dirs]
            for f in filenames:
                _, ext = os.path.splitext(f)
                if ext in exts:
                    rel = os.path.relpath(os.path.join(dirpath, f), self.project_root)
                    files.append(rel)
        return sorted(files)

    def collect_node_text(self, node, source_bytes: bytes) -> str:
        """Extract text of a tree-sitter node from source bytes."""
        return source_bytes[node.start_byte:node.end_byte].decode("utf-8", errors="replace")

    def _get_doc_comment_above(self, node, source_bytes: bytes, source_lines: list[str]) -> str:
        """Collect doc comments immediately above a node."""
        # Get the line before the node
        if node.start_point[0] == 0:
            return ""
        docs = []
        line_idx = node.start_point[0] - 1
        while line_idx >= 0:
            line = source_lines[line_idx].rstrip("\n")
            # Check for doc comment patterns (varies by language, subclasses override)
            stripped = line.strip()
            if stripped.startswith("///") or stripped.startswith("//!"):
                docs.insert(0, stripped.lstrip("/").strip())
                line_idx -= 1
            elif stripped.startswith("#") and not stripped.startswith("#!") and not stripped.startswith("##"):
                # Python type: comments don't count as doc
                break
            elif stripped.startswith("//"):
                docs.insert(0, stripped.lstrip("/").strip())
                line_idx -= 1
            elif stripped == "":
                line_idx -= 1  # skip blank lines
            else:
                break
        return " ".join(docs)


__all__ = ["BaseExtractor"]
