"""Project repository — project CRUD and queries."""

from typing import Any

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
        cur: Any = self._cursor()
        cur.execute(
            """INSERT INTO projects (name, primary_language, repo_path)
               VALUES (%s, %s, %s)
               ON CONFLICT (repo_path) DO UPDATE SET
                   name = EXCLUDED.name,
                   primary_language = EXCLUDED.primary_language,
                   updated_at = NOW()
               RETURNING id""",
            (name, language, repo_path),
        )
        pid: str = cur.fetchone()[0]
        self._commit()
        cur.close()
        return pid

    def get_by_path(self, repo_path: str) -> tuple[str, str, str, str] | None:
        """Get the project identified by its canonical repository path."""
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path "
            "FROM projects WHERE repo_path = %s",
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
        """Create a non-visible revision while its evidence is being built."""
        cur: Any = self._cursor()
        cur.execute(
            """INSERT INTO project_revisions
               (project_id, manifest_hash, file_count, signature_count,
                parser_version, status)
               VALUES (%s, %s, %s, %s, %s, 'building')
               RETURNING id""",
            (
                project_id,
                manifest_hash,
                file_count,
                signature_count,
                parser_version,
            ),
        )
        revision_id: str = cur.fetchone()[0]
        self._commit()
        cur.close()
        return revision_id

    def publish_revision(self, project_id: str, revision_id: str) -> None:
        """Atomically mark a completed revision ready and make it current.

        Construct this repository with ``auto_commit=False`` and perform all
        evidence reconciliation in the same ``repository_transaction`` to
        guarantee that readers never observe a partial revision.
        """
        cur: Any = self._cursor()
        cur.execute(
            """UPDATE project_revisions
               SET status = 'ready', published_at = NOW(), error_message = NULL
               WHERE id = %s AND project_id = %s AND status = 'building'
               RETURNING id""",
            (revision_id, project_id),
        )
        if cur.fetchone() is None:
            cur.close()
            raise ValueError("Revision is missing, belongs to another project, or is not building")
        cur.execute(
            """UPDATE projects
               SET current_revision_id = %s, updated_at = NOW()
               WHERE id = %s
               RETURNING id""",
            (revision_id, project_id),
        )
        if cur.fetchone() is None:
            cur.close()
            raise ValueError("Project does not exist")
        # Evidence tables hold only the current materialized index, not
        # historical snapshots. Retaining superseded "ready" rows would imply
        # that obsolete revisions remain queryable when they do not.
        cur.execute(
            "DELETE FROM project_revisions WHERE project_id = %s AND id <> %s",
            (project_id, revision_id),
        )
        self._commit()
        cur.close()

    def fail_revision(self, revision_id: str, error_message: str) -> bool:
        """Mark an unpublished revision failed without changing current data."""
        cur: Any = self._cursor()
        cur.execute(
            """UPDATE project_revisions
               SET status = 'failed', error_message = %s
               WHERE id = %s AND status = 'building'""",
            (error_message[:4000], revision_id),
        )
        changed = int(cur.rowcount) == 1
        self._commit()
        cur.close()
        return changed

    def get_current_revision(self, project_id: str) -> dict[str, Any] | None:
        """Return only the ready revision currently published for a project."""
        cur: Any = self._cursor()
        cur.execute(
            """SELECT r.id, r.manifest_hash, r.file_count, r.signature_count,
                      r.parser_version, r.published_at
               FROM projects p
               JOIN project_revisions r ON r.id = p.current_revision_id
               WHERE p.id = %s AND r.status = 'ready'""",
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
        """Purge one project and all direct or indirect evidence rows.

        Some legacy foreign keys intentionally use ``NO ACTION`` or ``SET
        NULL``. Delete in dependency order so ``/slugaudit off`` remains a
        complete purge on both upgraded and newly created databases.
        """
        cur: Any = self._cursor()
        cur.execute(
            "UPDATE projects SET config_id = NULL, current_revision_id = NULL "
            "WHERE id = %s",
            (project_id,),
        )
        cur.execute(
            """DELETE FROM file_staleness
               WHERE file_id IN (SELECT id FROM files WHERE project_id = %s)
                  OR source_file_id IN (SELECT id FROM files WHERE project_id = %s)
                  OR run_id IN (SELECT id FROM audit_runs WHERE project_id = %s)""",
            (project_id, project_id, project_id),
        )
        cur.execute(
            """DELETE FROM static_tool_results
               WHERE file_id IN (SELECT id FROM files WHERE project_id = %s)
                  OR run_id IN (SELECT id FROM audit_runs WHERE project_id = %s)""",
            (project_id, project_id),
        )
        cur.execute(
            """DELETE FROM ingestor_rejections
               WHERE run_id IN (SELECT id FROM audit_runs WHERE project_id = %s)""",
            (project_id,),
        )
        for table_name in (
            "architecture_state",
            "findings",
            "dependency_edges",
            "file_imports",
            "risk_patterns",
            "audit_runs",
            "audit_configs",
            "files",
            "project_revisions",
        ):
            # Identifiers are hardcoded above; values remain parameterized.
            query = f"DELETE FROM {table_name} WHERE project_id = %s"  # noqa: S608
            cur.execute(query, (project_id,))
        cur.execute("DELETE FROM projects WHERE id = %s RETURNING id", (project_id,))
        deleted = cur.fetchone() is not None
        self._commit()
        cur.close()
        return deleted

    def purge_by_path(self, repo_path: str) -> bool:
        """Purge one project by its canonical repository path."""
        cur: Any = self._cursor()
        cur.execute("SELECT id FROM projects WHERE repo_path = %s", (repo_path,))
        row = cur.fetchone()
        cur.close()
        if row is None:
            return False
        return self.purge_project(row[0])

    def get_by_name(self, name: str) -> tuple[str, str, str, str] | None:
        """Get project by name.

        Args:
            name: Project name.

        Returns:
            Tuple of (id, name, language, repo_path) or None.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects WHERE name = %s",
            (name,),
        )
        row: tuple[str, str, str, str] | None = cur.fetchone()
        cur.close()
        return row

    def get_latest(self) -> tuple[str, str, str, str] | None:
        """Get the most recently created project.

        Returns:
            Tuple of (id, name, language, repo_path) or None.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects "
            "ORDER BY created_at DESC LIMIT 1"
        )
        row: tuple[str, str, str, str] | None = cur.fetchone()
        cur.close()
        return row

    def get_all(self) -> list[tuple[str, str, str, str]]:
        """Get all projects.

        Returns:
            List of tuples (id, name, language, repo_path).
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, name, primary_language, repo_path FROM projects ORDER BY created_at DESC"
        )
        rows: list[tuple[str, str, str, str]] = cur.fetchall()
        cur.close()
        return rows

    def get_names(self) -> list[str]:
        """Get all project names.

        Returns:
            List of project names.
        """
        cur: Any = self._cursor()
        cur.execute("SELECT name FROM projects ORDER BY name")
        rows = [r[0] for r in cur.fetchall()]
        cur.close()
        return rows

    def get_status(self, project_id: str) -> dict[str, int]:
        """Get status summary for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Dict with file_count, total_size, signatures_count, imports_count, edge_count.
        """
        cur: Any = self._cursor()

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

    def get_findings_summary(self, project_id: str) -> list[tuple[int, str]]:
        """Get findings grouped by status.

        Args:
            project_id: Project UUID.

        Returns:
            List of (count, status) tuples.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT COUNT(*), status FROM findings "
            "WHERE project_id = %s GROUP BY status",
            (project_id,),
        )
        rows: list[tuple[int, str]] = cur.fetchall()
        cur.close()
        return rows

    def schema_exists(self) -> bool:
        """Check if the required database schema exists.

        Returns:
            True if the 'projects' table exists, False otherwise.
        """
        cur: Any = self._cursor()
        try:
            cur.execute(
                "SELECT EXISTS (SELECT FROM information_schema.tables "
                "WHERE table_schema = 'public' AND table_name = 'projects')"
            )
            row = cur.fetchone()
            return row[0] if row else False
        except Exception:
            return False
        finally:
            cur.close()


__all__ = ["ProjectRepository"]
