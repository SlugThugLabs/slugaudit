"""File repository — file CRUD, changed detection, stats."""

import json
from datetime import datetime, timezone
from typing import List, Optional, Set, Tuple

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
        signatures: list,
        force: bool = False,
    ) -> Tuple[str, bool]:
        """Upsert a file record.

        Args:
            project_id: Project UUID.
            relpath: Relative path from project root.
            file_hash: SHA-256 hash of file contents.
            file_size: File size in bytes.
            mtime: Last modification time (ISO format string).
            signatures: List of signature dicts for JSON cache.
            force: If True, update even if hash matches.

        Returns:
            Tuple of (file_id, was_updated).
        """
        cur = self._cursor()

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
                   signature_cache = %s, updated_at = NOW()
                   WHERE id = %s""",
                (file_hash, file_size, mtime, sig_json, fid),
            )
        else:
            cur.execute(
                """INSERT INTO files
                   (project_id, path, hash, size, last_modified_at, signature_cache)
                   VALUES (%s, %s, %s, %s, %s, %s)
                   RETURNING id""",
                (project_id, relpath, file_hash, file_size, mtime, sig_json),
            )
            fid = cur.fetchone()[0]

        self.conn.commit()
        cur.close()
        return fid, True

    def get_changed(self, project_id: str) -> List[Tuple[str, str]]:
        """Get files where hash differs from last_audited_hash.

        Args:
            project_id: Project UUID.

        Returns:
            List of (id, path) tuples.
        """
        cur = self._cursor()
        cur.execute(
            """SELECT id, path FROM files
               WHERE project_id = %s
               AND (last_audited_hash IS NULL OR hash != last_audited_hash)
               ORDER BY path""",
            (project_id,),
        )
        rows = cur.fetchall()
        cur.close()
        return rows

    def get_all_paths(self, project_id: str) -> List[Tuple[str, str]]:
        """Get all file IDs and paths for a project.

        Args:
            project_id: Project UUID.

        Returns:
            List of (id, path) tuples.
        """
        cur = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
        rows = cur.fetchall()
        cur.close()
        return rows

    def delete_removed(self, project_id: str, active_paths: Set[str]) -> int:
        """Remove files from DB that no longer exist on disk.

        Args:
            project_id: Project UUID.
            active_paths: Set of relative paths that still exist.

        Returns:
            Number of files deleted.
        """
        # Validated list of tables with their file_id column names
        tables_to_clean = (
            ("dependency_edges", "file_id", "source_file_id"),
            ("file_imports", "file_id", None),
            ("file_staleness", "file_id", "source_file_id"),
        )

        cur = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
        deleted = 0
        for fid, path in cur.fetchall():
            if path not in active_paths:
                for table_name, col1, col2 in tables_to_clean:
                    if col2:
                        cur.execute(
                            f"DELETE FROM {table_name} WHERE {col1} = %s OR {col2} = %s",
                            (fid, fid),
                        )
                    else:
                        cur.execute(
                            f"DELETE FROM {table_name} WHERE {col1} = %s",
                            (fid,),
                        )
                cur.execute("DELETE FROM findings WHERE file_id = %s", (fid,))
                cur.execute("DELETE FROM files WHERE id = %s", (fid,))
                deleted += 1

        self.conn.commit()
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
        self.conn.commit()
        cur.close()

    def get_file_stats(self, project_id: str) -> dict:
        """Get file statistics for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Dict with total_files, total_bytes, files_with_sigs, total_sigs.
        """
        cur = self._cursor()

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
        self, project_id: str, exclude_paths: Optional[List[str]] = None
    ) -> List[Tuple[str, list]]:
        """Get unchanged files with signature caches.

        Args:
            project_id: Project UUID.
            exclude_paths: Optional list of paths to exclude (target files).

        Returns:
            List of (path, signature_cache) tuples.
        """
        cur = self._cursor()

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

    def get_blast_radius(self, project_id: str, changed_paths: List[str]) -> Set[str]:
        """Get files that depend on changed files.

        Args:
            project_id: Project UUID.
            changed_paths: List of changed file paths.

        Returns:
            Set of file paths that depend on changed files.
        """
        if not changed_paths:
            return set()

        cur = self._cursor()
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
