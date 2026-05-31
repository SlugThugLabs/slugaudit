"""Briefing assembler — coordinates providers and formatters."""

import os
from typing import Optional

from infrastructure import get_connection, get_file_system, IFileSystem
from repositories import ProjectRepository

# Backward-compat: expose get_changed_files for test patching
from db import get_changed_files

from .providers import (
    OverviewProvider,
    ArchitectureProvider,
    ChangedFilesProvider,
    GhostContextProvider,
    TargetFileProvider,
    FindingsProvider,
)
from .formatter import (
    format_header,
    format_overview,
    format_architecture,
    format_ghost_context,
    format_target_files,
    format_findings,
    format_output_contract,
)


class BriefingAssembler:
    """Assembles a structured audit briefing from database and filesystem."""

    def __init__(
        self,
        project_name: Optional[str] = None,
        output_path: Optional[str] = None,
        connection_str: Optional[str] = None,
        max_ghost_lines: int = 500,
        file_system: Optional[IFileSystem] = None,
    ):
        """Create a briefing assembler.

        Args:
            project_name: Project name (default: latest).
            output_path: Path to write output (default: return string).
            connection_str: PostgreSQL connection string.
            max_ghost_lines: Maximum ghost context lines.
            file_system: File system abstraction.
        """
        self.project_name = project_name
        self.output_path = output_path
        self.connection_str = connection_str
        self.max_ghost_lines = max_ghost_lines
        self.file_system = file_system or get_file_system()

    def assemble(self) -> Optional[str]:
        """Assemble the briefing and optionally write to file.

        Returns:
            The briefing string, or None if project not found.
        """
        conn = None
        try:
            conn = get_connection(self.connection_str)
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
            ghost = GhostContextProvider(conn, project_id, self.max_ghost_lines)
            targets = TargetFileProvider(conn, project_id, repo_path, self.file_system)
            findings_prov = FindingsProvider(conn, project_id)

            # --- Gather data ---
            stats = overview.get_stats()
            architecture_text = arch.get_text()
            changed_paths = changed.get_changed_paths()
            blast_radius = changed.get_blast_radius(changed_paths)
            target_paths = sorted(set(changed_paths) | blast_radius)

            unchanged_with_sigs = ghost.get_unchanged_with_sigs(
                target_paths if target_paths else None
            )
            findings_list = findings_prov.get_open_findings()

            # --- Format sections ---
            lines = []
            lines.extend(format_header(name, changed_paths, language))
            lines.extend(format_overview(
                language, stats, len(changed_paths), len(blast_radius), len(findings_list)
            ))
            lines.extend(format_architecture(architecture_text, language))
            lines.extend(format_ghost_context(
                unchanged_with_sigs, stats["total_sigs"], self.max_ghost_lines
            ))
            lines.extend(format_target_files(
                changed_paths, blast_radius, target_paths,
                repo_path, language, self.file_system,
            ))
            lines.extend(format_findings(findings_list))
            lines.extend(format_output_contract())

            briefing = "\n".join(lines)

            # --- Output ---
            if self.output_path:
                with open(self.output_path, 'w') as f:
                    f.write(briefing)
                print(f"Briefing written to {self.output_path}")
                print(f"  {stats['total_files']} files, {stats['total_sigs']} signatures, "
                      f"{len(target_paths)} targets")
                print(f"  Ghost context: {len(unchanged_with_sigs)} files")
                print(f"  Briefing size: {len(briefing)} chars / ~{len(briefing.split())} words")

            return briefing

        finally:
            if conn:
                conn.close()


def assemble_briefing(
    project_name: Optional[str] = None,
    output_path: Optional[str] = None,
    connection_str: Optional[str] = None,
    max_ghost_lines: int = 500,
) -> Optional[str]:
    """Assemble the audit briefing from the database.

    This is the standalone function entry point for backward compatibility.
    """
    assembler = BriefingAssembler(
        project_name=project_name,
        output_path=output_path,
        connection_str=connection_str,
        max_ghost_lines=max_ghost_lines,
    )
    return assembler.assemble()


__all__ = ["BriefingAssembler", "assemble_briefing"]
