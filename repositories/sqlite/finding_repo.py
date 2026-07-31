"""SQLite finding repository — sibling of repositories/finding_repo.py."""

import uuid
from typing import Any

from ..base import BaseRepository

_NOW = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"


class SqliteFindingRepository(BaseRepository):
    """SQLite-backed findings data access."""

    def get_open_findings(
        self, project_id: str, limit: int = 50
    ) -> list[tuple[str, int | None, int | None, str, str, str]]:
        cur: Any = self._cursor()
        cur.execute(
            """
            SELECT f.path, fi.line_start, fi.line_end, fi.severity,
                   fi.category, fi.message
            FROM findings fi
            JOIN files f ON f.id = fi.file_id
            WHERE fi.project_id = ? AND fi.status = 'open'
            ORDER BY fi.created_at DESC
            LIMIT ?
            """,
            (project_id, limit),
        )
        rows: list[tuple[str, int | None, int | None, str, str, str]] = cur.fetchall()
        cur.close()
        return rows

    def record(
        self,
        *,
        project_id: str,
        file_id: str,
        identity_hash: str,
        severity: str,
        category: str,
        message: str,
        line_start: int | None = None,
        line_end: int | None = None,
    ) -> tuple[str, bool]:
        """Create or refresh one AI finding identity for current evidence."""
        cur: Any = self._cursor()
        cur.execute(
            "SELECT id FROM findings "
            "WHERE project_id = ? AND identity_hash = ? "
            "ORDER BY created_at LIMIT 1",
            (project_id, identity_hash),
        )
        row = cur.fetchone()
        if row is not None:
            finding_id = row[0]
            cur.execute(
                f"""UPDATE findings SET file_id = ?, severity = ?,
                          category = ?, message = ?, line_start = ?,
                          line_end = ?, status = 'open', updated_at = {_NOW}
                   WHERE id = ?""",  # noqa: S608 - {_NOW} is a fixed constant, never input
                (file_id, severity, category, message, line_start, line_end, finding_id),
            )
            created = False
        else:
            finding_id = str(uuid.uuid4())
            cur.execute(
                """INSERT INTO findings
                   (id, project_id, file_id, identity_hash, severity, category,
                    message, line_start, line_end, status, triage_source)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'open', 'ai')""",
                (
                    finding_id,
                    project_id,
                    file_id,
                    identity_hash,
                    severity,
                    category,
                    message,
                    line_start,
                    line_end,
                ),
            )
            created = True
        self._commit()
        cur.close()
        return finding_id, created


__all__ = ["SqliteFindingRepository"]
