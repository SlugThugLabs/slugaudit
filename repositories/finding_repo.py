"""Finding repository — findings CRUD and queries."""


from typing import Any

from .base import BaseRepository


class FindingRepository(BaseRepository):
    """Repository for findings data access."""

    def get_open_findings(
        self, project_id: str, limit: int = 50
    ) -> list[tuple[str, int | None, int | None, str, str, str]]:
        """Get open findings for a project.

        Args:
            project_id: Project UUID.
            limit: Maximum number of findings to return.

        Returns:
            List of (path, line_start, line_end, severity, category, message) tuples.
        """
        cur: Any = self._cursor()
        cur.execute(
            """
            SELECT f.path, fi.line_start, fi.line_end, fi.severity,
                   fi.category, fi.message
            FROM findings fi
            JOIN files f ON f.id = fi.file_id
            WHERE fi.project_id = %s AND fi.status = 'open'
            ORDER BY fi.created_at DESC
            LIMIT %s
            """,
            (project_id, limit),
        )
        rows: list[tuple[str, int | None, int | None, str, str, str]] = cur.fetchall()
        cur.close()
        return rows

    def get_summary(self, project_id: str) -> list[tuple[int, str]]:
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

    def insert(
        self,
        project_id: str,
        file_id: str,
        identity_hash: str,
        severity: str,
        category: str,
        message: str,
        line_start: int | None = None,
        line_end: int | None = None,
        blast_radius: int | None = None,
        proximity: int | None = None,
        risk_score: float | None = None,
        status: str = "open",
    ) -> str:
        """Insert a finding.

        Args:
            project_id: Project UUID.
            file_id: File UUID.
            identity_hash: Hash of finding identity for deduplication.
            severity: Severity level (low, medium, high, critical).
            category: Finding category.
            message: Finding description.
            line_start: Start line (1-based).
            line_end: End line (1-based).
            blast_radius: Number of files in blast radius.
            proximity: Proximity score.
            risk_score: Calculated risk score.
            status: Finding status (open, resolved, false_positive).

        Returns:
            Finding UUID.
        """
        cur: Any = self._cursor()
        cur.execute(
            """INSERT INTO findings
               (project_id, file_id, identity_hash, severity, category, message,
                line_start, line_end, blast_radius, proximity, risk_score, status)
               VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
               RETURNING id""",
            (
                project_id,
                file_id,
                identity_hash,
                severity,
                category,
                message,
                line_start,
                line_end,
                blast_radius,
                proximity,
                risk_score,
                status,
            ),
        )
        fid: str = cur.fetchone()[0]
        self._commit()
        cur.close()
        return fid

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
            "WHERE project_id = %s AND identity_hash = %s "
            "ORDER BY created_at LIMIT 1",
            (project_id, identity_hash),
        )
        row = cur.fetchone()
        if row is not None:
            finding_id = row[0]
            cur.execute(
                """UPDATE findings SET file_id = %s, severity = %s,
                          category = %s, message = %s, line_start = %s,
                          line_end = %s, status = 'open', updated_at = NOW()
                   WHERE id = %s""",
                (
                    file_id,
                    severity,
                    category,
                    message,
                    line_start,
                    line_end,
                    finding_id,
                ),
            )
            created = False
        else:
            cur.execute(
                """INSERT INTO findings
                   (project_id, file_id, identity_hash, severity, category,
                    message, line_start, line_end, status, triage_source)
                   VALUES (%s, %s, %s, %s, %s, %s, %s, %s, 'open', 'ai')
                   RETURNING id""",
                (
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
            finding_id = cur.fetchone()[0]
            created = True
        self._commit()
        cur.close()
        return finding_id, created


__all__ = ["FindingRepository"]
