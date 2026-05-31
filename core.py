"""
core.py — Shared audit logic used by both CLI and MCP server.

This module contains the core import pipeline: scanning, extraction,
upsert, dependency graph construction, and cleanup. Both the CLI
(audit_db.py) and the MCP server (mcp_server.py) delegate to these
functions, eliminating duplication.
"""

import os
import sys
from datetime import datetime, timezone
from typing import Optional, List, Dict, Any

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from db import (
    get_connection,
    get_or_create_project,
    upsert_file,
    delete_removed_files,
    insert_imports,
    build_dependency_edges,
    update_audit_timestamps,
)
from languages import detect_language, list_languages
from languages.rust import RustExtractor
from languages.python import PythonExtractor
from languages.typescript import TypeScriptExtractor

LANG_MAP = {
    "rust": RustExtractor,
    "python": PythonExtractor,
    "typescript": TypeScriptExtractor,
}


def get_extractor(project_root: str, language: Optional[str]):
    """Resolve the extractor class for the given language and project root.

    Args:
        project_root: Absolute path to the project directory.
        language: Language name ("rust", "python", "typescript") or "auto".

    Returns:
        An extractor instance.

    Raises:
        ValueError: If the language cannot be detected or is unsupported.
    """
    if language and language != "auto":
        cls = LANG_MAP.get(language)
        if not cls:
            raise ValueError(
                f"Unsupported language: {language}. Supported: {list_languages()}"
            )
        return cls(project_root)

    cls = detect_language(project_root)
    if cls:
        return cls(project_root)

    for name, cls in LANG_MAP.items():
        ext = cls.source_extensions()
        for root, dirs, files in os.walk(project_root):
            dirs[:] = [
                d for d in dirs
                if not d.startswith(".") and d not in (
                    "target", "node_modules", "venv", "__pycache__", "build", "dist"
                )
            ]
            for f in files:
                if any(f.endswith(e) for e in ext):
                    return cls(project_root)

    raise ValueError(
        "Could not detect language. Specify with language parameter: "
        "rust, python, or typescript"
    )


class ImportResult:
    """Result of an import operation."""

    def __init__(self):
        self.project_id: Optional[str] = None
        self.project_name: str = ""
        self.language: str = ""
        self.files_processed: int = 0
        self.signatures_extracted: int = 0
        self.imports_extracted: int = 0
        self.dependency_edges: int = 0
        self.elapsed_seconds: float = 0.0

    def __str__(self) -> str:
        return (
            f"Import complete for project: {self.project_name}\n"
            f"  ID: {self.project_id}\n"
            f"  Language: {self.language}\n"
            f"  Files processed: {self.files_processed}\n"
            f"  Signatures extracted: {self.signatures_extracted}\n"
            f"  Imports extracted: {self.imports_extracted}\n"
            f"  Dependency edges: {self.dependency_edges}\n"
            f"  Elapsed: {self.elapsed_seconds:.1f}s"
        )


def import_project(
    project_path: str,
    project_name: Optional[str] = None,
    language: Optional[str] = "auto",
    connection_string: Optional[str] = None,
    on_progress: Optional[callable] = None,
) -> ImportResult:
    """Import a project into the audit database.

    This is the single import entry point used by both the CLI and MCP server.
    It handles the full pipeline: scanning, extraction, upsert, dependency graph,
    cleanup, and timestamp updates.

    Args:
        project_path: Absolute path to the project directory.
        project_name: Project name (default: directory basename).
        language: Language to use for extraction, or "auto" to detect.
        connection_string: PostgreSQL connection string. If None, uses env vars.
        on_progress: Optional callback(int processed, int total) for progress.

    Returns:
        ImportResult with statistics about the import.

    Raises:
        ValueError: If project_path is not a directory.
        Exception: Propagates database or extraction errors.
    """
    project_root = os.path.abspath(project_path)

    if not os.path.isdir(project_root):
        raise ValueError(f"Not a directory: {project_path}")

    extractor = get_extractor(project_root, language)
    conn = None

    try:
        conn = get_connection(connection_string)
        name = project_name or os.path.basename(project_root)

        start = datetime.now()
        project_id = get_or_create_project(
            conn,
            name=name,
            repo_path=project_root,
            language=extractor.name(),
        )

        rel_paths = extractor.find_source_files()
        total_sigs = 0
        total_imports = 0

        for i, relpath in enumerate(rel_paths):
            file_path = os.path.join(project_root, relpath)

            try:
                with open(file_path, "rb") as f:
                    content = f.read()

                file_hash = extractor.hash_file(file_path)
                file_size = len(content)
                mtime = datetime.fromtimestamp(
                    os.path.getmtime(file_path), tz=timezone.utc
                ).isoformat()

                sigs = extractor.extract_signatures(file_path, content)
                imps = extractor.extract_imports(file_path, content)

                sig_cache = _build_sig_cache(sigs)

                file_id, _ = upsert_file(
                    conn,
                    project_id=project_id,
                    relpath=relpath,
                    file_hash=file_hash,
                    file_size=file_size,
                    mtime=mtime,
                    signatures=sig_cache,
                )

                total_sigs += len(sigs)
                total_imports += len(imps)

                if imps:
                    import_records = _build_import_records(imps)
                    insert_imports(conn, project_id, file_id, import_records)

            except Exception as e:
                # Log but don't fail the entire import for a single file error
                if on_progress:
                    on_progress(i + 1, len(rel_paths), str(e))
                continue

            if on_progress:
                on_progress(i + 1, len(rel_paths))

        # Build dependency graph
        edges = 0
        try:
            edges = build_dependency_edges(conn, project_id, extractor)
        except Exception:
            pass  # Dependency graph failure is non-fatal

        # Clean up removed files
        delete_removed_files(conn, project_id, set(rel_paths))

        # Mark all files as audited
        update_audit_timestamps(conn, project_id)

        elapsed = (datetime.now() - start).total_seconds()

        result = ImportResult()
        result.project_id = project_id
        result.project_name = name
        result.language = extractor.name()
        result.files_processed = len(rel_paths)
        result.signatures_extracted = total_sigs
        result.imports_extracted = total_imports
        result.dependency_edges = edges
        result.elapsed_seconds = elapsed

        return result

    finally:
        if conn:
            try:
                conn.close()
            except Exception:
                pass


def _build_sig_cache(sigs: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Convert raw signature dicts into the JSON-cacheable format."""
    result = []
    for sig in sigs:
        entry = {
            "type": sig.get("type", "unknown"),
            "name": sig.get("name", ""),
            "visibility": sig.get("visibility", ""),
            "signature": sig.get("signature", ""),
        }
        if sig.get("generic_params"):
            entry["generic_params"] = sig["generic_params"]
        result.append(entry)
    return result


def _build_import_records(imps: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Convert raw import dicts into database import records."""
    return [
        {
            "import_text": imp["import_text"],
            "resolved_path": None,
            "import_type": imp.get("import_type", "internal"),
            "line_start": imp.get("line_start"),
            "line_end": imp.get("line_end"),
        }
        for imp in imps
    ]
