"""SQLite file repository — sibling of repositories/file_repo.py.

Scoped to only the methods the current tool surface actually calls (upsert,
get_manifest, purge_obsolete_findings, delete_removed, update_audit_timestamps,
get_file_stats, get_all_paths_ordered, search_by_pattern, get_file_contents,
get_dependents, get_file_identity) — see CLAUDE.md for why this is smaller
than the PostgreSQL repository.
"""

import json
import uuid
from typing import Any

from ..base import BaseRepository

_NOW = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"


class SqliteFileRepository(BaseRepository):
    """SQLite-backed file data access."""

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
        cur: Any = self._cursor()

        cur.execute(
            "SELECT id, hash FROM files WHERE project_id = ? AND path = ?",
            (project_id, relpath),
        )
        row = cur.fetchone()

        fid: str
        if row and not force:
            fid, existing_hash = str(row[0]), row[1]
            if existing_hash == file_hash:
                cur.close()
                return fid, False

        sig_json = json.dumps(signatures) if signatures else None

        if row:
            fid = str(row[0])
            cur.execute(
                f"""UPDATE files SET
                   hash = ?, size = ?, last_modified_at = ?,
                   signature_cache = ?, content = COALESCE(?, content),
                   updated_at = {_NOW}
                   WHERE id = ?""",  # noqa: S608 - {_NOW} is a fixed constant, never input
                (file_hash, file_size, mtime, sig_json, content, fid),
            )
        else:
            fid = str(uuid.uuid4())
            cur.execute(
                """INSERT INTO files
                   (id, project_id, path, hash, size, last_modified_at,
                    signature_cache, content)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                (fid, project_id, relpath, file_hash, file_size, mtime, sig_json, content),
            )

        self._commit()
        cur.close()
        return fid, True

    def get_manifest(self, project_id: str) -> dict[str, str]:
        cur: Any = self._cursor()
        cur.execute(
            "SELECT path, hash FROM files WHERE project_id = ? ORDER BY path",
            (project_id,),
        )
        manifest = dict(cur.fetchall())
        cur.close()
        return manifest

    def purge_obsolete_findings(self, project_id: str, file_id: str) -> int:
        cur: Any = self._cursor()
        cur.execute(
            "DELETE FROM findings WHERE project_id = ? AND file_id = ?",
            (project_id, file_id),
        )
        deleted = int(cur.rowcount)
        self._commit()
        cur.close()
        return deleted

    def delete_removed(self, project_id: str, active_paths: set[str]) -> int:
        cur: Any = self._cursor()
        cur.execute("SELECT id, path FROM files WHERE project_id = ?", (project_id,))
        removed_ids = [fid for fid, path in cur.fetchall() if path not in active_paths]
        deleted = len(removed_ids)
        if removed_ids:
            placeholders = ", ".join("?" for _ in removed_ids)
            # dependency_edges/file_imports/findings all reference files with
            # ON DELETE CASCADE/SET NULL (sqlite_schema.sql); PRAGMA
            # foreign_keys=ON (infrastructure/sqlite_db.py) makes a plain
            # DELETE FROM files cascade correctly with no explicit cleanup.
            cur.execute(
                f"DELETE FROM files WHERE id IN ({placeholders})",  # noqa: S608
                removed_ids,
            )
        self._commit()
        cur.close()
        return deleted

    def update_audit_timestamps(self, project_id: str) -> None:
        cur: Any = self._cursor()
        cur.execute(
            "UPDATE files SET last_audited_hash = hash WHERE project_id = ?",
            (project_id,),
        )
        self._commit()
        cur.close()

    def get_file_stats(self, project_id: str) -> dict[str, int]:
        cur: Any = self._cursor()

        cur.execute(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE project_id = ?",
            (project_id,),
        )
        total_files, total_bytes = cur.fetchone()

        # signature_cache is JSON text; computed here in Python rather than
        # relying on SQLite's optional JSON1 extension, for maximum
        # portability across SQLite builds.
        cur.execute(
            "SELECT signature_cache FROM files "
            "WHERE project_id = ? AND signature_cache IS NOT NULL",
            (project_id,),
        )
        files_with_sigs = 0
        total_sigs = 0
        for (cache_text,) in cur.fetchall():
            try:
                sigs = json.loads(cache_text) if cache_text else []
            except (TypeError, ValueError):
                sigs = []
            if sigs:
                files_with_sigs += 1
                total_sigs += len(sigs)

        cur.close()
        return {
            "total_files": int(total_files),
            "total_bytes": int(total_bytes),
            "files_with_sigs": files_with_sigs,
            "total_sigs": total_sigs,
        }

    def search_by_pattern(
        self,
        project_id: str,
        pattern: str,
        is_regex: bool,
        max_results: int,
    ) -> list[tuple[str, str | None]]:
        cur: Any = self._cursor()
        if is_regex:
            # REGEXP dispatches to the function registered in
            # infrastructure/sqlite_db.py:connect(), which applies the same
            # per-call timeout as the PostgreSQL path's statement_timeout.
            cur.execute(
                "SELECT path, content FROM files WHERE project_id = ? "
                "AND content REGEXP ? ORDER BY path LIMIT ?",
                (project_id, pattern, max_results),
            )
        else:
            # SQLite's LIKE is already case-insensitive for ASCII, matching
            # ILIKE's behavior without needing a separate operator.
            escaped = (
                pattern.replace("\\", "\\\\")
                .replace("%", "\\%")
                .replace("_", "\\_")
            )
            cur.execute(
                "SELECT path, content FROM files WHERE project_id = ? "
                "AND content LIKE ? ESCAPE '\\' ORDER BY path LIMIT ?",
                (project_id, f"%{escaped}%", max_results),
            )
        rows = cur.fetchall()
        cur.close()
        return rows  # type: ignore[no-any-return]

    def get_file_contents(
        self, project_id: str, paths: list[str]
    ) -> list[tuple[str, str | None]]:
        cur: Any = self._cursor()
        placeholders = ", ".join("?" for _ in paths)
        cur.execute(
            f"SELECT path, content FROM files WHERE project_id = ? "  # noqa: S608
            f"AND path IN ({placeholders}) ORDER BY path",
            (project_id, *paths),
        )
        rows = cur.fetchall()
        cur.close()
        return rows  # type: ignore[no-any-return]

    def get_file_identity(self, project_id: str, path: str) -> tuple[str, str] | None:
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id, hash FROM files WHERE project_id = ? AND path = ?",
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
        cur: Any = self._cursor()
        if direction == "incoming":
            cur.execute(
                """
                SELECT DISTINCT f2.path FROM dependency_edges de
                JOIN files f1 ON f1.id = de.target_file_id
                JOIN files f2 ON f2.id = de.source_file_id
                WHERE de.project_id = ? AND f1.path = ?
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
                WHERE de.project_id = ? AND f1.path = ?
                ORDER BY f2.path
                """,
                (project_id, file_path),
            )
        paths = [row[0] for row in cur.fetchall()]
        cur.close()
        return paths

    def get_all_paths_ordered(self, project_id: str) -> list[str]:
        cur: Any = self._cursor()
        cur.execute(
            "SELECT path FROM files WHERE project_id = ? ORDER BY path",
            (project_id,),
        )
        paths = [row[0] for row in cur.fetchall()]
        cur.close()
        return paths


__all__ = ["SqliteFileRepository"]
