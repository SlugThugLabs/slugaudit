"""Briefing service — orchestrates briefing assembly."""


from typing import Any

from briefing import BriefingAssembler


class BriefingService:
    """Service for generating audit briefings."""

    def __init__(self, file_system: Any = None) -> None:
        """Create a briefing service.

        Args:
            file_system: File system abstraction. Defaults to LocalFileSystem.
        """
        self.file_system = file_system

    def generate(
        self,
        project_name: str | None = None,
        output_path: str | None = None,
        connection_str: str | None = None,
        max_ghost_lines: int = 500,
    ) -> str | None:
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
