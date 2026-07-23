"""Briefing assembler — coordinates providers and formatters."""

import os
from typing import Any

from infrastructure import get_connection, get_file_system, IFileSystem
from repositories import ProjectRepository

# Backward-compat: expose get_changed_files for test patching
# get_changed_files available via repositories.FileRepository

from .providers import (
    OverviewProvider,
    ArchitectureProvider,
    ChangedFilesProvider,
    FindingsProvider,
    RiskPatternsProvider,
)
from .formatter import (
    format_header,
    format_overview,
    format_architecture,
    format_target_files,
    format_findings,
    format_risk_patterns,
)


class BriefingAssembler:
    """Assembles a structured audit briefing from database and filesystem."""

    def __init__(
        self,
        project_name: str | None = None,
        output_path: str | None = None,
        connection_str: str | None = None,
        max_ghost_lines: int = 500,
        file_system: IFileSystem | None = None,
        connection: Any = None,
    ):
        """Create a briefing assembler.

        Args:
            project_name: Project name (default: latest).
            output_path: Path to write output (default: return string).
            connection_str: PostgreSQL connection string (used if connection not provided).
            max_ghost_lines: Maximum ghost context lines.
            file_system: File system abstraction.
            connection: Optional existing DB connection (overrides connection_str).
        """
        self.project_name = project_name
        self.output_path = output_path
        self.connection_str = connection_str
        self.max_ghost_lines = max_ghost_lines
        self.file_system = file_system or get_file_system()
        self.connection = connection

    def assemble(self) -> str | None:
        """Assemble the briefing and optionally write to file.

        Returns:
            The briefing string, or None if project not found.
        """
        conn = None
        try:
            conn = self.connection or get_connection(self.connection_str)
            project_repo = ProjectRepository(conn)

            # --- Get project ---
            if self.project_name:
                row = project_repo.get_by_name(self.project_name)
            else:
                row = project_repo.get_latest()

            if not row:
                print("No project found.")
                return None

            project_id, name, language, repo_path = row

            # --- Validate repo_path ---
            if not os.path.isdir(repo_path):
                print(f"Warning: repo path not found: {repo_path}")

            # --- Providers ---
            overview = OverviewProvider(conn, project_id)
            arch = ArchitectureProvider(conn, project_id)
            changed = ChangedFilesProvider(conn, project_id)
            findings_prov = FindingsProvider(conn, project_id)
            risk_prov = RiskPatternsProvider(conn, project_id)

            # --- Gather data ---
            stats = overview.get_stats()
            architecture_text = arch.get_text()
            changed_paths = changed.get_changed_paths()
            blast_radius = changed.get_blast_radius(changed_paths)
            target_paths = sorted(set(changed_paths) | blast_radius)

            findings_list = findings_prov.get_open_findings()
            file_patterns = risk_prov.get_file_patterns()
            risk_summary = risk_prov.get_summary()

            # --- Format sections ---
            lines: list[Any] = []
            lines.extend(format_header(name, changed_paths, language))
            lines.extend(format_overview(
                language, stats, len(changed_paths), len(blast_radius), len(findings_list)
            ))
            lines.extend(format_architecture(architecture_text, language))
            lines.extend(format_risk_patterns(file_patterns, risk_summary))
            lines.extend(format_target_files(
                changed_paths, blast_radius, target_paths,
                repo_path, language, self.file_system,
            ))
            lines.extend(format_findings(findings_list))

            briefing = "\n".join(lines)

            # --- Output ---
            if self.output_path:
                with open(self.output_path, 'w') as f:
                    f.write(briefing)
                print(f"Briefing written to {self.output_path}")
                print(f"  {stats['total_files']} files, {stats['total_sigs']} signatures, "
                      f"{len(target_paths)} targets")
                print(f"  Briefing size: {len(briefing)} chars / ~{len(briefing.split())} words")

            return briefing

        finally:
            if conn is not None and conn is not self.connection:
                conn.close()


def assemble_briefing(
    project_name: str | None = None,
    output_path: str | None = None,
    connection_str: str | None = None,
    max_ghost_lines: int = 500,
    connection: object | None = None,
) -> str | None:
    """Assemble the audit briefing from the database.

    Args:
        project_name: Project name (default: latest).
        output_path: Path to write output (default: return string).
        connection_str: PostgreSQL connection string (fallback).
        max_ghost_lines: Maximum ghost context lines.
        connection: Optional existing DB connection (overrides connection_str).
    """
    assembler = BriefingAssembler(
        project_name=project_name,
        output_path=output_path,
        connection_str=connection_str,
        max_ghost_lines=max_ghost_lines,
        connection=connection,
    )
    return assembler.assemble()


__all__ = ["BriefingAssembler", "assemble_briefing"]
