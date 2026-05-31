"""Finding repository — findings CRUD and queries."""

from typing import List, Optional, Tuple

from .base import BaseRepository


class FindingRepository(BaseRepository):
    """Repository for findings data access."""

    def get_open_findings(
        self, project_id: str, limit: int = 50
    ) -> List[Tuple[str, Optional[int], Optional[int], str, str, str]]:
        """Get open findings for a project.

        Args:
            project_id: Project UUID.
            limit: Maximum number of findings to return.

        Returns:
            List of (path, line_start, line_end, severity, category, message) tuples.
        """
        cur = self._cursor()
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
        rows = cur.fetchall()
        cur.close()
        return rows

    def get_summary(self, project_id: str) -> List[Tuple[int, str]]:
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

    def insert(
        self,
        project_id: str,
        file_id: str,
        identity_hash: str,
        severity: str,
        category: str,
        message: str,
        line_start: Optional[int] = None,
        line_end: Optional[int] = None,
        blast_radius: Optional[int] = None,
        proximity: Optional[int] = None,
        risk_score: Optional[float] = None,
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
        cur = self._cursor()
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
        fid = cur.fetchone()[0]
        self.conn.commit()
        cur.close()
        return fid


__all__ = ["FindingRepository"]
