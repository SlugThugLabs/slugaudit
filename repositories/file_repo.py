"""File repository — file CRUD, changed detection, stats."""

import json
from typing import Any

from .base import BaseRepository


class FileRepository(BaseRepository):
    """Repository for file data access."""

    def upsert(
        self,
        project_id: str,
        relpath: str,
        file_hash: str,
        file_size: int,
        mtime: str,
        signatures: list[dict[str, Any]],
        content: str | None = None,
        force: bool = False,
    ) -> tuple[str, bool]:
        """Upsert a file record.

        Args:
            project_id: Project UUID.
            relpath: Relative path from project root.
            file_hash: SHA-256 hash of file contents.
            file_size: File size in bytes.
            mtime: Last modification time (ISO format string).
            signatures: List of signature dicts for JSON cache.
            content: Optional file content text (stored for search/read operations).
            force: If True, update even if hash matches.

        Returns:
            Tuple of (file_id, was_updated).
        """
        cur: Any = self._cursor()

        cur.execute(
            "SELECT id, hash FROM files WHERE project_id = %s AND path = %s",
            (project_id, relpath),
        )
        row = cur.fetchone()

        if row and not force:
            fid, existing_hash = row
            if existing_hash == file_hash:
                cur.close()
                return fid, False

        sig_json = json.dumps(signatures) if signatures else None

        if row:
            fid = row[0]
            cur.execute(
                """UPDATE files SET
                   hash = %s, size = %s, last_modified_at = %s,
                   signature_cache = %s, content = COALESCE(%s, content), updated_at = NOW()
                   WHERE id = %s""",
                (file_hash, file_size, mtime, sig_json, content, fid),
            )
        else:
            cur.execute(
                """INSERT INTO files
                   (project_id, path, hash, size, last_modified_at, signature_cache, content)
                   VALUES (%s, %s, %s, %s, %s, %s, %s)
                   RETURNING id""",
                (project_id, relpath, file_hash, file_size, mtime, sig_json, content),
            )
            fid = cur.fetchone()[0]

        self._commit()
        cur.close()
        return fid, True

    def get_changed(self, project_id: str) -> list[tuple[str, str]]:
        """Get files where hash differs from last_audited_hash.

        Args:
            project_id: Project UUID.

        Returns:
            List of (id, path) tuples.
        """
        cur: Any = self._cursor()
        cur.execute(
            """SELECT id, path FROM files
               WHERE project_id = %s
               AND (last_audited_hash IS NULL OR hash != last_audited_hash)
               ORDER BY path""",
            (project_id,),
        )
        rows: list[tuple[str, str]] = cur.fetchall()
        cur.close()
        return rows

    def get_all_paths(self, project_id: str) -> list[tuple[str, str]]:
        """Get all file IDs and paths for a project.

        Args:
            project_id: Project UUID.

        Returns:
            List of (id, path) tuples.
        """
        cur: Any = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
        rows: list[tuple[str, str]] = cur.fetchall()
        cur.close()
        return rows

    def get_manifest(self, project_id: str) -> dict[str, str]:
        """Return the currently indexed path-to-content-hash manifest."""
        cur: Any = self._cursor()
        cur.execute(
            "SELECT path, hash FROM files WHERE project_id = %s ORDER BY path",
            (project_id,),
        )
        manifest = dict(cur.fetchall())
        cur.close()
        return manifest

    def purge_obsolete_findings(self, project_id: str, file_id: str) -> int:
        """Delete findings whose source evidence is being replaced."""
        cur: Any = self._cursor()
        cur.execute(
            "DELETE FROM findings WHERE project_id = %s AND file_id = %s",
            (project_id, file_id),
        )
        deleted = int(cur.rowcount)
        self._commit()
        cur.close()
        return deleted

    def delete_removed(self, project_id: str, active_paths: set[str]) -> int:
        """Remove files from DB that no longer exist on disk.

        Args:
            project_id: Project UUID.
            active_paths: Set of relative paths that still exist.

        Returns:
            Number of files deleted.
        """
        cur: Any = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
        removed_ids = [fid for fid, path in cur.fetchall() if path not in active_paths]
        deleted = len(removed_ids)
        if removed_ids:
            # Most derived rows cascade from files. These explicit deletes also
            # remove rows whose secondary source_file_id uses ON DELETE SET NULL,
            # and findings whose file FK intentionally uses SET NULL.
            cur.execute(
                "DELETE FROM dependency_edges "
                "WHERE source_file_id = ANY(%s::uuid[]) "
                "OR target_file_id = ANY(%s::uuid[])",
                (removed_ids, removed_ids),
            )
            cur.execute(
                "DELETE FROM file_imports WHERE file_id = ANY(%s::uuid[])",
                (removed_ids,),
            )
            cur.execute(
                "DELETE FROM file_staleness "
                "WHERE file_id = ANY(%s::uuid[]) "
                "OR source_file_id = ANY(%s::uuid[])",
                (removed_ids, removed_ids),
            )
            cur.execute(
                "DELETE FROM findings WHERE file_id = ANY(%s::uuid[])",
                (removed_ids,),
            )
            cur.execute(
                "DELETE FROM files WHERE id = ANY(%s::uuid[])", (removed_ids,)
            )

        self._commit()
        cur.close()
        return deleted

    def update_audit_timestamps(self, project_id: str) -> None:
        """Update last_audited_hash to match current hash for all files.

        Args:
            project_id: Project UUID.
        """
        cur = self._cursor()
        cur.execute(
            "UPDATE files SET last_audited_hash = hash WHERE project_id = %s",
            (project_id,),
        )
        self._commit()
        cur.close()

    def get_file_stats(self, project_id: str) -> dict[str, int]:
        """Get file statistics for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Dict with total_files, total_bytes, files_with_sigs, total_sigs.
        """
        cur: Any = self._cursor()

        cur.execute(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE project_id = %s",
            (project_id,),
        )
        total_files, total_bytes = cur.fetchone()

        cur.execute(
            "SELECT COUNT(*) FROM files WHERE project_id = %s "
            "AND signature_cache IS NOT NULL AND jsonb_array_length(signature_cache) > 0",
            (project_id,),
        )
        files_with_sigs = cur.fetchone()[0]

        total_sigs = 0
        cur.execute(
            "SELECT COALESCE(SUM(jsonb_array_length(signature_cache)), 0) "
            "FROM files WHERE project_id = %s AND signature_cache IS NOT NULL",
            (project_id,),
        )
        result = cur.fetchone()
        if result:
            total_sigs = result[0]

        cur.close()

        return {
            "total_files": total_files,
            "total_bytes": total_bytes,
            "files_with_sigs": files_with_sigs,
            "total_sigs": total_sigs,
        }

    def get_unchanged_with_sigs(
        self, project_id: str, exclude_paths: list[str] | None = None
    ) -> list[tuple[str, list[dict[str, Any]]]]:
        """Get unchanged files with signature caches.

        Args:
            project_id: Project UUID.
            exclude_paths: Optional list of paths to exclude (target files).

        Returns:
            List of (path, signature_cache) tuples.
        """
        cur: Any = self._cursor()

        if exclude_paths:
            cur.execute(
                """
                SELECT path, signature_cache
                FROM files
                WHERE project_id = %s
                  AND signature_cache IS NOT NULL
                  AND jsonb_array_length(signature_cache) > 0
                  AND path != ALL(%s)
                ORDER BY path
                """,
                (project_id, exclude_paths),
            )
        else:
            cur.execute(
                """
                SELECT path, signature_cache
                FROM files
                WHERE project_id = %s
                  AND signature_cache IS NOT NULL
                  AND jsonb_array_length(signature_cache) > 0
                ORDER BY path
                """,
                (project_id,),
            )

        rows = cur.fetchall()
        cur.close()
        return [(path, list(sig_cache)) for path, sig_cache in rows]

    def search_by_pattern(
        self,
        project_id: str,
        pattern: str,
        is_regex: bool,
        max_results: int,
    ) -> list[tuple[str, str | None]]:
        """Search file contents by pattern.

        Args:
            project_id: Project UUID.
            pattern: Search pattern (substring or regex).
            is_regex: If True, use regex matching; otherwise case-insensitive ILIKE.
            max_results: Maximum number of files to return.

        Returns:
            List of (path, content) tuples.
        """
        cur: Any = self._cursor()
        if is_regex:
            cur.execute("SET LOCAL statement_timeout = '3000ms'")
            cur.execute(
                "SELECT path, content FROM files WHERE project_id = %s "
                "AND content ~* %s ORDER BY path LIMIT %s",
                (project_id, pattern, max_results),
            )
        else:
            escaped = (
                pattern.replace("\\", "\\\\")
                .replace("%", "\\%")
                .replace("_", "\\_")
            )
            cur.execute(
                "SELECT path, content FROM files WHERE project_id = %s "
                "AND content ILIKE %s ESCAPE '\\' ORDER BY path LIMIT %s",
                (project_id, f"%{escaped}%", max_results),
            )
        rows = cur.fetchall()
        cur.close()
        return rows  # type: ignore[no-any-return]

    def get_file_contents(
        self, project_id: str, paths: list[str]
    ) -> list[tuple[str, str | None]]:
        """Get file contents for specific paths.

        Args:
            project_id: Project UUID.
            paths: List of relative file paths.

        Returns:
            List of (path, content) tuples, ordered by path.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT path, content FROM files WHERE project_id = %s "
            "AND path = ANY(%s) ORDER BY path",
            (project_id, paths),
        )
        rows = cur.fetchall()
        cur.close()
        return rows  # type: ignore[no-any-return]

    def get_file_identity(self, project_id: str, path: str) -> tuple[str, str] | None:
        """Return a file ID and current content hash for one indexed path."""
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, hash FROM files WHERE project_id = %s AND path = %s",
            (project_id, path),
        )
        row: tuple[str, str] | None = cur.fetchone()
        cur.close()
        return row

    def get_dependents(
        self,
        project_id: str,
        file_path: str,
        direction: str = "incoming",
    ) -> list[str]:
        """Get files that depend on a file (incoming) or are imported by it (outgoing).

        Args:
            project_id: Project UUID.
            file_path: The file path to query.
            direction: "incoming" (blast radius) or "outgoing" (imports).

        Returns:
            List of file paths.
        """
        cur: Any = self._cursor()
        if direction == "incoming":
            cur.execute(
                """
                SELECT DISTINCT f2.path FROM dependency_edges de
                JOIN files f1 ON f1.id = de.target_file_id
                JOIN files f2 ON f2.id = de.source_file_id
                WHERE de.project_id = %s AND f1.path = %s
                ORDER BY f2.path
                """,
                (project_id, file_path),
            )
        else:
            cur.execute(
                """
                SELECT DISTINCT f2.path FROM dependency_edges de
                JOIN files f1 ON f1.id = de.source_file_id
                JOIN files f2 ON f2.id = de.target_file_id
                WHERE de.project_id = %s AND f1.path = %s
                ORDER BY f2.path
                """,
                (project_id, file_path),
            )
        paths = [row[0] for row in cur.fetchall()]
        cur.close()
        return paths

    def get_all_paths_ordered(self, project_id: str) -> list[str]:
        """Get all file paths for a project, ordered alphabetically.

        Args:
            project_id: Project UUID.

        Returns:
            List of file paths.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT path FROM files WHERE project_id = %s ORDER BY path",
            (project_id,),
        )
        paths = [row[0] for row in cur.fetchall()]
        cur.close()
        return paths

    def get_blast_radius(self, project_id: str, changed_paths: list[str]) -> set[str]:
        """Get files that depend on changed files.

        Args:
            project_id: Project UUID.
            changed_paths: List of changed file paths.

        Returns:
            Set of file paths that depend on changed files.
        """
        if not changed_paths:
            return set()

        cur: Any = self._cursor()
        cur.execute(
            """
            SELECT DISTINCT f2.path
            FROM dependency_edges de
            JOIN files f1 ON f1.id = de.source_file_id
            JOIN files f2 ON f2.id = de.target_file_id
            WHERE de.project_id = %s
              AND f1.path = ANY(%s)
            """,
            (project_id, changed_paths),
        )
        paths = {row[0] for row in cur.fetchall()}
        cur.close()
        return paths


__all__ = ["FileRepository"]
