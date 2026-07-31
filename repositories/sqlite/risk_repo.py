"""SQLite risk pattern repository — sibling of repositories/risk_repo.py."""

import uuid
from typing import Any

from ..base import BaseRepository


class SqliteRiskPatternRepository(BaseRepository):
    """SQLite-backed risk pattern detection results."""

    def upsert(self, project_id: str, file_id: str, patterns: list[dict[str, Any]]) -> int:
        cur: Any = self._cursor()
        cur.execute("DELETE FROM risk_patterns WHERE file_id = ?", (file_id,))

        count = 0
        for pat in patterns:
            cur.execute(
                """INSERT INTO risk_patterns (id, project_id, file_id, pattern_type, count)
                   VALUES (?, ?, ?, ?, ?)""",
                (str(uuid.uuid4()), project_id, file_id, pat["pattern_type"], pat["count"]),
            )
            count += 1

        self._commit()
        cur.close()
        return count

    def get_project_patterns(
        self, project_id: str
    ) -> list[tuple[str, list[tuple[str, int]]]]:
        cur: Any = self._cursor()
        cur.execute(
            """SELECT f.path, rp.pattern_type, rp.count
               FROM risk_patterns rp
               JOIN files f ON rp.file_id = f.id
               WHERE rp.project_id = ?
               ORDER BY f.path, rp.count DESC""",
            (project_id,),
        )
        rows = cur.fetchall()
        cur.close()

        result: dict[str, list[tuple[str, int]]] = {}
        for path, pattern_type, count in rows:
            if path not in result:
                result[path] = []
            result[path].append((pattern_type, count))

        return sorted(result.items())

    def get_pattern_summary(self, project_id: str) -> dict[str, int]:
        cur: Any = self._cursor()
        cur.execute(
            """SELECT pattern_type, SUM(count) as total
               FROM risk_patterns
               WHERE project_id = ?
               GROUP BY pattern_type
               ORDER BY total DESC""",
            (project_id,),
        )
        rows = cur.fetchall()
        cur.close()
        return {r[0]: r[1] for r in rows}


__all__ = ["SqliteRiskPatternRepository"]
