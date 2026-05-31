"""Architecture repository — architecture state queries."""

from typing import Optional, Tuple

from .base import BaseRepository


class ArchitectureRepository(BaseRepository):
    """Repository for architecture state data access."""

    def get_latest(self, project_id: str) -> Optional[Tuple[str, Optional[str]]]:
        """Get the latest architecture state for a project.

        Args:
            project_id: Project UUID.

        Returns:
            Tuple of (summary, layer_map) or None.
        """
        cur = self._cursor()
        cur.execute(
            "SELECT summary, layer_map FROM architecture_state "
            "WHERE project_id = %s ORDER BY created_at DESC LIMIT 1",
            (project_id,),
        )
        row = cur.fetchone()
        cur.close()
        return row


__all__ = ["ArchitectureRepository"]
