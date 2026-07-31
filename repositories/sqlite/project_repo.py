"""SQLite project repository — sibling of repositories/project_repo.py.

Same method names/signatures as ProjectRepository, translated to SQLite:
UUIDs generated here in Python (no gen_random_uuid()), ``?`` placeholders,
plain ints from SQLite's driver (no Decimal-from-SUM(bigint) class of bug —
SQLite has no numeric/Decimal concept at all). Scoped to only the methods
the current tool surface actually calls; see CLAUDE.md.
"""

import json
import uuid
from typing import Any

from ..base import BaseRepository

_NOW = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"


class SqliteProjectRepository(BaseRepository):
    """SQLite-backed project data access."""

    def get_or_create(self, name: str, language: str, repo_path: str) -> str:
        cur: Any = self._cursor()
        cur.execute("SELECT id FROM projects WHERE repo_path = ?", (repo_path,))
        row = cur.fetchone()
        project_id: str
        if row:
            project_id = str(row[0])
            cur.execute(
                f"UPDATE projects SET name = ?, primary_language = ?, "  # noqa: S608
                f"updated_at = {_NOW} WHERE id = ?",
                (name, language, project_id),
            )
        else:
            project_id = str(uuid.uuid4())
            cur.execute(
                "INSERT INTO projects (id, name, primary_language, repo_path) "
                "VALUES (?, ?, ?, ?)",
                (project_id, name, language, repo_path),
            )
        self._commit()
        cur.close()
        return project_id

    def get_by_path(self, repo_path: str) -> tuple[str, str, str, str] | None:
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects "
            "WHERE repo_path = ?",
            (repo_path,),
        )
        row: tuple[str, str, str, str] | None = cur.fetchone()
        cur.close()
        return row

    def begin_revision(
        self,
        project_id: str,
        manifest_hash: str,
        file_count: int,
        signature_count: int = 0,
        parser_version: str | None = None,
    ) -> str:
        cur: Any = self._cursor()
        revision_id = str(uuid.uuid4())
        cur.execute(
            "INSERT INTO project_revisions "
            "(id, project_id, manifest_hash, file_count, signature_count, "
            " parser_version, status) VALUES (?, ?, ?, ?, ?, ?, 'building')",
            (revision_id, project_id, manifest_hash, file_count, signature_count, parser_version),
        )
        self._commit()
        cur.close()
        return revision_id

    def publish_revision(self, project_id: str, revision_id: str) -> None:
        cur: Any = self._cursor()
        cur.execute(
            f"UPDATE project_revisions SET status = 'ready', "  # noqa: S608
            f"published_at = {_NOW}, error_message = NULL "
            f"WHERE id = ? AND project_id = ? AND status = 'building'",
            (revision_id, project_id),
        )
        if cur.rowcount == 0:
            cur.close()
            raise ValueError(
                "Revision is missing, belongs to another project, or is not building"
            )
        cur.execute(
            f"UPDATE projects SET current_revision_id = ?, updated_at = {_NOW} "  # noqa: S608
            f"WHERE id = ?",
            (revision_id, project_id),
        )
        if cur.rowcount == 0:
            cur.close()
            raise ValueError("Project does not exist")
        cur.execute(
            "DELETE FROM project_revisions WHERE project_id = ? AND id <> ?",
            (project_id, revision_id),
        )
        self._commit()
        cur.close()

    def get_current_revision(self, project_id: str) -> dict[str, Any] | None:
        cur: Any = self._cursor()
        cur.execute(
            """SELECT r.id, r.manifest_hash, r.file_count, r.signature_count,
                      r.parser_version, r.published_at
               FROM projects p
               JOIN project_revisions r ON r.id = p.current_revision_id
               WHERE p.id = ? AND r.status = 'ready'""",
            (project_id,),
        )
        row = cur.fetchone()
        cur.close()
        if row is None:
            return None
        return {
            "revision_id": row[0],
            "manifest_hash": row[1],
            "file_count": row[2],
            "signature_count": row[3],
            "parser_version": row[4],
            "published_at": row[5],
        }

    def purge_project(self, project_id: str) -> bool:
        """Purge one project and all its evidence rows (foreign keys cascade)."""
        cur: Any = self._cursor()
        cur.execute("DELETE FROM projects WHERE id = ?", (project_id,))
        deleted = bool(cur.rowcount > 0)
        self._commit()
        cur.close()
        return deleted

    def purge_by_path(self, repo_path: str) -> bool:
        cur: Any = self._cursor()
        cur.execute("SELECT id FROM projects WHERE repo_path = ?", (repo_path,))
        row = cur.fetchone()
        cur.close()
        if row is None:
            return False
        return self.purge_project(row[0])

    def get_status(self, project_id: str) -> dict[str, int]:
        cur: Any = self._cursor()
        cur.execute(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE project_id = ?",
            (project_id,),
        )
        file_count, total_size = cur.fetchone()

        # signature_cache is stored as JSON text (no JSONB/json_array_length
        # dependency for maximum SQLite-build portability); computed here
        # rather than in SQL.
        cur.execute(
            "SELECT signature_cache FROM files "
            "WHERE project_id = ? AND signature_cache IS NOT NULL",
            (project_id,),
        )
        with_sigs = 0
        total_sigs = 0
        for (cache_text,) in cur.fetchall():
            try:
                sigs = json.loads(cache_text) if cache_text else []
            except (TypeError, ValueError):
                sigs = []
            if sigs:
                with_sigs += 1
                total_sigs += len(sigs)

        cur.execute(
            "SELECT COUNT(*) FROM file_imports WHERE project_id = ?", (project_id,)
        )
        import_count = cur.fetchone()[0]

        cur.execute(
            "SELECT COUNT(*) FROM dependency_edges WHERE project_id = ?", (project_id,)
        )
        edge_count = cur.fetchone()[0]

        cur.close()
        return {
            "file_count": int(file_count),
            "total_size": int(total_size),
            "files_with_sigs": with_sigs,
            "signatures_count": total_sigs,
            "imports_count": int(import_count),
            "edge_count": int(edge_count),
        }


__all__ = ["SqliteProjectRepository"]
