"""Import service — project import orchestration."""

import logging
import os
from datetime import datetime, timezone
from typing import Callable, List, Dict, Any, Optional

from domain import ImportResult
from infrastructure import get_connection, get_file_system, IFileSystem
from repositories import FileRepository, ImportRepository, ProjectRepository


logger = logging.getLogger(__name__)


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


class ImportService:
    """Orchestrates project import into the audit database."""

    def __init__(
        self,
        extractor,
        file_system: Optional[IFileSystem] = None,
    ):
        """Create an import service.

        Args:
            extractor: Language extractor instance.
            file_system: File system abstraction. Defaults to LocalFileSystem.
        """
        self.extractor = extractor
        self.file_system = file_system or get_file_system()

    def import_project(
        self,
        project_path: str,
        project_name: Optional[str] = None,
        language: Optional[str] = "auto",
        connection_string: Optional[str] = None,
        on_progress: Optional[Callable] = None,
    ) -> ImportResult:
        """Import a project into the audit database.

        Args:
            project_path: Absolute path to the project directory.
            project_name: Project name (default: directory basename).
            language: Language to use for extraction, or "auto" to detect.
            connection_string: PostgreSQL connection string. If None, uses env vars.
            on_progress: Optional callback(processed, total) or (processed, total, error_msg).

        Returns:
            ImportResult with statistics about the import.

        Raises:
            ValueError: If project_path is not a directory.
            Exception: Propagates database or extraction errors.
        """
        project_root = os.path.abspath(project_path)

        if not os.path.isdir(project_root):
            raise ValueError(f"Not a directory: {project_path}")

        conn = None

        try:
            conn = get_connection(connection_string)
            name = project_name or os.path.basename(project_root)

            start = datetime.now()

            # Repositories
            project_repo = ProjectRepository(conn)
            file_repo = FileRepository(conn)
            import_repo = ImportRepository(conn)

            project_id = project_repo.get_or_create(
                name=name,
                repo_path=project_root,
                language=self.extractor.name(),
            )

            rel_paths = self.extractor.find_source_files()
            total_sigs = 0
            total_imports = 0

            for i, relpath in enumerate(rel_paths):
                file_path = os.path.join(project_root, relpath)

                try:
                    content = self.file_system.read_file_bytes(file_path)

                    file_hash = self.extractor.hash_file(file_path)
                    file_size = len(content)
                    mtime = datetime.fromtimestamp(
                        self.file_system.get_mtime(file_path), tz=timezone.utc
                    ).isoformat()

                    sigs = self.extractor.extract_signatures(file_path, content)
                    imps = self.extractor.extract_imports(file_path, content)

                    sig_cache = _build_sig_cache(sigs)

                    file_id, _ = file_repo.upsert(
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
                        import_repo.insert(conn, project_id, file_id, import_records)

                except Exception as e:
                    if on_progress:
                        on_progress(i + 1, len(rel_paths), str(e))
                    continue

                if on_progress:
                    on_progress(i + 1, len(rel_paths))

                # Batch commits every 100 files for performance
                if (i + 1) % 100 == 0:
                    conn.commit()

            # Build dependency graph
            edges = 0
            try:
                edges = import_repo.build_dependency_edges(conn, project_id, self.extractor)
            except Exception as e:
                logger.warning("Failed to build dependency graph for %s: %s", name, e)

            # Clean up removed files
            file_repo.delete_removed(conn, project_id, set(rel_paths))

            # Mark all files as audited
            file_repo.update_audit_timestamps(conn, project_id)

            elapsed = (datetime.now() - start).total_seconds()

            result = ImportResult()
            result.project_id = project_id
            result.project_name = name
            result.language = self.extractor.name()
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


# Standalone function for backward compatibility
def import_project(
    project_path: str,
    project_name: Optional[str] = None,
    language: Optional[str] = "auto",
    connection_string: Optional[str] = None,
    on_progress: Optional[Callable] = None,
) -> ImportResult:
    """Import a project into the audit database.

    This is the single import entry point used by both the CLI and MCP server.
    """
    # Deferred imports to avoid circular dependencies
    from core import get_extractor

    extractor = get_extractor(project_path, language)
    service = ImportService(extractor)
    return service.import_project(
        project_path=project_path,
        project_name=project_name,
        language=language,
        connection_string=connection_string,
        on_progress=on_progress,
    )


__all__ = ["ImportService", "import_project"]
