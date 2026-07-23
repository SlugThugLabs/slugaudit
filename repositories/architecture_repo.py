"""Architecture repository — architecture state queries."""


from typing import Any

from .base import BaseRepository


class ArchitectureRepository(BaseRepository):
    """Repository for architecture state data access."""

    def get_latest(self, project_id: str) -> tuple[str, str | None] | None:
        """Get the latest architecture state for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Tuple of (summary, layer_map) or None.
        """
        cur: Any = self._cursor()
        cur.execute(
            "SELECT summary, layer_map FROM architecture_state "
            "WHERE project_id = %s ORDER BY created_at DESC LIMIT 1",
            (project_id,),
        )
        row: tuple[str, str | None] | None = cur.fetchone()
        cur.close()
        return row


__all__ = ["ArchitectureRepository"]
