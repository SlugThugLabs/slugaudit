"""Project repository — project CRUD and queries."""

from typing import List, Optional, Tuple

from .base import BaseRepository


class ProjectRepository(BaseRepository):
    """Repository for project data access."""

    def get_or_create(self, name: str, language: str, repo_path: str) -> str:
        """Get existing project by path or create a new one.

        Args:
            name: Project name.
            language: Primary language.
            repo_path: Absolute path to the project repository.

        Returns:
            Project UUID.
        """
        cur = self._cursor()
        cur.execute("SELECT id FROM projects WHERE repo_path = %s", (repo_path,))
        row = cur.fetchone()
        if row:
            pid = row[0]
            cur.execute(
                "UPDATE projects SET primary_language = %s, updated_at = NOW() WHERE id = %s",
                (language, pid),
            )
            cur.close()
            return pid

        cur.execute(
            "INSERT INTO projects (name, primary_language, repo_path) VALUES (%s, %s, %s) RETURNING id",
            (name, language, repo_path),
        )
        pid = cur.fetchone()[0]
        self.conn.commit()
        cur.close()
        return pid

    def get_by_name(self, name: str) -> Optional[Tuple[str, str, str, str]]:
        """Get project by name.

        Args:
            name: Project name.

        Returns:
            Tuple of (id, name, language, repo_path) or None.
        """
        cur = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects WHERE name = %s",
            (name,),
        )
        row = cur.fetchone()
        cur.close()
        return row

    def get_latest(self) -> Optional[Tuple[str, str, str, str]]:
        """Get the most recently created project.

        Returns:
            Tuple of (id, name, language, repo_path) or None.
        """
        cur = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects "
            "ORDER BY created_at DESC LIMIT 1"
        )
        row = cur.fetchone()
        cur.close()
        return row

    def get_all(self) -> List[Tuple[str, str, str, str]]:
        """Get all projects.

        Returns:
            List of tuples (id, name, language, repo_path).
        """
        cur = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects ORDER BY created_at DESC"
        )
        rows = cur.fetchall()
        cur.close()
        return rows

    def get_names(self) -> List[str]:
        """Get all project names.

        Returns:
            List of project names.
        """
        cur = self._cursor()
        cur.execute("SELECT name FROM projects ORDER BY name")
        rows = [r[0] for r in cur.fetchall()]
        cur.close()
        return rows

    def get_status(self, project_id: str) -> dict:
        """Get status summary for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Dict with file_count, total_size, signatures_count, imports_count, edge_count.
        """
        cur = self._cursor()

        cur.execute(
            "SELECT COUNT(*), COALESCE(SUM(size), 0), "
            "COUNT(*) FILTER (WHERE signature_cache IS NOT NULL "
            "AND jsonb_array_length(signature_cache) > 0) "
            "FROM files WHERE project_id = %s",
            (project_id,),
        )
        file_count, total_size, with_sigs = cur.fetchone()

        total_sigs = 0
        if with_sigs:
            cur.execute(
                "SELECT SUM(jsonb_array_length(signature_cache)) "
                "FROM files WHERE project_id = %s AND signature_cache IS NOT NULL",
                (project_id,),
            )
            total_sigs = (cur.fetchone())[0] or 0

        cur.execute(
            "SELECT COUNT(*) FROM file_imports WHERE project_id = %s",
            (project_id,),
        )
        import_count = cur.fetchone()[0]

        cur.execute(
            "SELECT COUNT(*) FROM dependency_edges WHERE project_id = %s",
            (project_id,),
        )
        edge_count = cur.fetchone()[0]

        cur.close()

        return {
            "file_count": file_count,
            "total_size": total_size,
            "files_with_sigs": with_sigs,
            "signatures_count": total_sigs,
            "imports_count": import_count,
            "edge_count": edge_count,
        }

    def get_findings_summary(self, project_id: str) -> List[Tuple[int, str]]:
        """Get findings grouped by status.

        Args:
            project_id: Project UUID.

        Returns:
            List of (count, status) tuples.
        """
        cur = self._cursor()
        cur.execute(
            "SELECT COUNT(*), status FROM findings "
            "WHERE project_id = %s GROUP BY status",
            (project_id,),
        )
        rows = cur.fetchall()
        cur.close()
        return rows


__all__ = ["ProjectRepository"]
