"""Risk pattern repository — store and query per-file risk pattern counts."""

import logging
from typing import Any

from .base import BaseRepository

logger = logging.getLogger(__name__)


class RiskPatternRepository(BaseRepository):
    """Repository for risk pattern detection results."""

    def upsert(self, project_id: str, file_id: str, patterns: list[dict[str, Any]]) -> int:
        """Store risk patterns for a file.

        Args:
            project_id: Project UUID.
            file_id: File UUID.
            patterns: List of {pattern_type, count} dicts.

        Returns:
            Number of patterns stored.
        """
        cur: Any = self._cursor()
        # Delete existing patterns for this file first
        cur.execute("DELETE FROM risk_patterns WHERE file_id = %s", (file_id,))

        count = 0
        for pat in patterns:
            cur.execute(
                """INSERT INTO risk_patterns (project_id, file_id, pattern_type, count)
                   VALUES (%s, %s, %s, %s)""",
                (project_id, file_id, pat["pattern_type"], pat["count"]),
            )
            count += 1

        self._commit()
        cur.close()
        return count

    def get_file_patterns(self, file_id: str) -> list[tuple[str, int]]:
        """Get risk patterns for a file.

        Args:
            file_id: File UUID.

        Returns:
            List of (pattern_type, count) tuples.
        """
        cur: Any = self._cursor()
        cur.execute(
            """SELECT pattern_type, count FROM risk_patterns
               WHERE file_id = %s ORDER BY count DESC""",
            (file_id,),
        )
        rows = cur.fetchall()
        cur.close()
        return [(r[0], r[1]) for r in rows]

    def get_project_patterns(self, project_id: str) -> list[tuple[str, list[tuple[str, int]]]]:
        """Get all risk patterns for a project, grouped by file.

        Args:
            project_id: Project UUID.

        Returns:
            List of (file_path, [(pattern_type, count), ...]) tuples.
        """
        cur: Any = self._cursor()
        cur.execute(
            """SELECT f.path, rp.pattern_type, rp.count
               FROM risk_patterns rp
               JOIN files f ON rp.file_id = f.id
               WHERE rp.project_id = %s
               ORDER BY f.path, rp.count DESC""",
            (project_id,),
        )
        rows = cur.fetchall()
        cur.close()

        # Group by file
        result: dict[str, list[tuple[str, int]]] = {}
        for path, pattern_type, count in rows:
            if path not in result:
                result[path] = []
            result[path].append((pattern_type, count))

        return sorted(result.items())

    def get_pattern_summary(self, project_id: str) -> dict[str, int]:
        """Get total counts per pattern type across the project.

        Args:
            project_id: Project UUID.

        Returns:
            Dict of {pattern_type: total_count}.
        """
        cur: Any = self._cursor()
        cur.execute(
            """SELECT pattern_type, SUM(count) as total
               FROM risk_patterns
               WHERE project_id = %s
               GROUP BY pattern_type
               ORDER BY total DESC""",
            (project_id,),
        )
        rows = cur.fetchall()
        cur.close()
        return {r[0]: r[1] for r in rows}


__all__ = ["RiskPatternRepository"]
