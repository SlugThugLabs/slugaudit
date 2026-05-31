"""Briefing service — orchestrates briefing assembly."""

from typing import Optional

from briefing import BriefingAssembler
from infrastructure import get_file_system


class BriefingService:
    """Service for generating audit briefings."""

    def __init__(self, file_system=None):
        """Create a briefing service.

        Args:
            file_system: File system abstraction. Defaults to LocalFileSystem.
        """
        self.file_system = file_system

    def generate(
        self,
        project_name: Optional[str] = None,
        output_path: Optional[str] = None,
        connection_str: Optional[str] = None,
        max_ghost_lines: int = 500,
    ) -> Optional[str]:
        """Generate an audit briefing.

        Args:
            project_name: Project name (default: latest).
            output_path: Path to write output (default: return string).
            connection_str: PostgreSQL connection string.
            max_ghost_lines: Maximum ghost context lines.

        Returns:
            The briefing string, or None if project not found.
        """
        assembler = BriefingAssembler(
            project_name=project_name,
            output_path=output_path,
            connection_str=connection_str,
            max_ghost_lines=max_ghost_lines,
            file_system=self.file_system,
        )
        return assembler.assemble()


__all__ = ["BriefingService"]
