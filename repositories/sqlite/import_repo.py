"""SQLite import repository — sibling of repositories/import_repo.py."""

import uuid
from typing import Any

from ..base import BaseRepository


class SqliteImportRepository(BaseRepository):
    """SQLite-backed import records and dependency edges."""

    def insert(
        self,
        project_id: str,
        file_id: str,
        imports: list[dict[str, Any]],
        force: bool = True,
    ) -> int:
        cur: Any = self._cursor()
        if force:
            cur.execute("DELETE FROM file_imports WHERE file_id = ?", (file_id,))

        count = 0
        for imp in imports:
            cur.execute(
                """INSERT INTO file_imports
                   (id, project_id, file_id, import_text, resolved_path,
                    import_type, line_start, line_end)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                (
                    str(uuid.uuid4()),
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

    def get_file_map(self, project_id: str) -> dict[str, str]:
        cur: Any = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = ?", (project_id,))
        path_to_id: dict[str, str] = {path: fid for fid, path in cur.fetchall()}
        cur.close()
        return path_to_id

    def get_unresolved_internal(self, project_id: str) -> list[tuple[str, str, str, str]]:
        cur: Any = self._cursor()
        cur.execute(
            """SELECT fi.id, fi.file_id, fi.import_text, f.path
               FROM file_imports fi
               JOIN files f ON f.id = fi.file_id
               WHERE fi.project_id = ? AND fi.import_type = 'internal'
               AND fi.resolved_path IS NULL""",
            (project_id,),
        )
        rows: list[tuple[str, str, str, str]] = cur.fetchall()
        cur.close()
        return rows

    def build_dependency_edges(
        self,
        project_id: str,
        importer: Any,
        force: bool = True,
    ) -> int:
        cur = self._cursor()
        if force:
            cur.execute("DELETE FROM dependency_edges WHERE project_id = ?", (project_id,))
            cur.execute(
                "UPDATE file_imports SET resolved_path = NULL WHERE project_id = ?",
                (project_id,),
            )

        path_to_id: dict[str, str] = self.get_file_map(project_id)
        rows: list[tuple[str, str, str, str]] = self.get_unresolved_internal(project_id)

        edges_added = 0
        for import_id, src_file_id, import_text, src_path in rows:
            resolved = importer.resolve_import(import_text, src_path, path_to_id)
            if resolved and resolved in path_to_id:
                target_id: str = path_to_id[resolved]
                if target_id != src_file_id:
                    cur.execute(
                        """INSERT INTO dependency_edges
                           (id, project_id, source_file_id, target_file_id, import_id)
                           VALUES (?, ?, ?, ?, ?)
                           ON CONFLICT (source_file_id, target_file_id, import_id)
                           DO NOTHING""",
                        (str(uuid.uuid4()), project_id, src_file_id, target_id, import_id),
                    )
                    if cur.rowcount > 0:
                        edges_added += 1
                cur.execute(
                    "UPDATE file_imports SET resolved_path = ? WHERE id = ?",
                    (resolved, import_id),
                )

        self._commit()
        cur.close()
        return edges_added


__all__ = ["SqliteImportRepository"]
