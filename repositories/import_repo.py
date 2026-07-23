"""Import repository — import records and dependency edges."""

import logging
from typing import Any

from .base import BaseRepository

logger = logging.getLogger(__name__)


class ImportRepository(BaseRepository):
    """Repository for import records and dependency edges."""

    def insert(
        self,
        project_id: str,
        file_id: str,
        imports: list[dict[str, Any]],
        force: bool = True,
    ) -> int:
        """Insert import records for a file.

        Args:
            project_id: Project UUID.
            file_id: File UUID.
            imports: List of import record dicts.
            force: Replace existing imports first. Defaults to True so a file
                that removed all imports cannot leave stale dependency facts.

        Returns:
            Number of imports inserted.
        """
        cur: Any = self._cursor()
        if force:
            cur.execute("DELETE FROM file_imports WHERE file_id = %s", (file_id,))

        count = 0
        for imp in imports:
            cur.execute(
                """INSERT INTO file_imports
                   (project_id, file_id, import_text, resolved_path,
                    import_type, line_start, line_end)
                   VALUES (%s, %s, %s, %s, %s, %s, %s)""",
                (
                    project_id,
                    file_id,
                    imp["import_text"],
                    imp.get("resolved_path"),
                    imp.get("import_type", "internal"),
                    imp.get("line_start"),
                    imp.get("line_end"),
                ),
            )
            count += 1

        self._commit()
        cur.close()
        return count

    def get_unresolved_internal(self, project_id: str) -> list[tuple[str, str, str, str]]:
        """Get all unresolved internal imports for a project.

        Args:
            project_id: Project UUID.

        Returns:
            List of (import_id, file_id, import_text, source_path) tuples.
        """
        cur: Any = self._cursor()
        cur.execute(
            """SELECT fi.id, fi.file_id, fi.import_text, f.path
               FROM file_imports fi
               JOIN files f ON f.id = fi.file_id
               WHERE fi.project_id = %s AND fi.import_type = 'internal'
               AND fi.resolved_path IS NULL""",
            (project_id,),
        )
        rows: list[tuple[str, str, str, str]] = cur.fetchall()
        cur.close()
        return rows

    def get_file_map(self, project_id: str) -> dict[str, str]:
        """Get mapping of file paths to file IDs for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Dict mapping path → file_id.
        """
        cur: Any = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
        path_to_id: dict[str, str] = {path: fid for fid, path in cur.fetchall()}
        cur.close()
        return path_to_id

    def build_dependency_edges(
        self,
        project_id: str,
        importer: Any,
        force: bool = True,
    ) -> int:
        """Resolve all file_imports to dependency edges between files.

        Args:
            project_id: Project UUID.
            importer: Extractor instance with resolve_import() method.
            force: Rebuild existing project edges first. Defaults to True.

        Returns:
            Number of edges added.
        """
        cur = self._cursor()
        if force:
            cur.execute("DELETE FROM dependency_edges WHERE project_id = %s", (project_id,))

        # Build path → id mapping
        path_to_id: dict[str, str] = self.get_file_map(project_id)

        # Get unresolved imports
        rows: list[tuple[str, str, str, str]] = self.get_unresolved_internal(project_id)

        edges_added = 0
        for import_id, src_file_id, import_text, src_path in rows:
            resolved = importer.resolve_import(import_text, src_path, path_to_id)
            if resolved and resolved in path_to_id:
                target_id: str = path_to_id[resolved]
                if target_id != src_file_id:
                    cur.execute(
                        """INSERT INTO dependency_edges
                           (project_id, source_file_id, target_file_id, import_id)
                           VALUES (%s, %s, %s, %s)
                           ON CONFLICT (source_file_id, target_file_id, import_id)
                           DO NOTHING""",
                        (project_id, src_file_id, target_id, import_id),
                    )
                    if cur.rowcount > 0:
                        edges_added += 1
                cur.execute(
                    "UPDATE file_imports SET resolved_path = %s WHERE id = %s",
                    (resolved, import_id),
                )

        self._commit()
        cur.close()
        return edges_added

    def get_import_count(self, project_id: str) -> int:
        """Get total import count for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Number of import records.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT COUNT(*) FROM file_imports WHERE project_id = %s",
            (project_id,),
        )
        count: int = cur.fetchone()[0]
        cur.close()
        return count

    def get_edge_count(self, project_id: str) -> int:
        """Get total dependency edge count for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Number of dependency edges.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT COUNT(*) FROM dependency_edges WHERE project_id = %s",
            (project_id,),
        )
        count: int = cur.fetchone()[0]
        cur.close()
        return count


__all__ = ["ImportRepository"]
