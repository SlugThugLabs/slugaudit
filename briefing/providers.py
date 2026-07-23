"""Briefing data providers — each handles one data source."""


from typing import Any

from repositories import (
    FileRepository,
    ImportRepository,
    FindingRepository,
    ArchitectureRepository,
    RiskPatternRepository,
)


class OverviewProvider:
    """Provides project overview statistics."""

    def __init__(self, conn: Any, project_id: str):
        self.conn = conn
        self.project_id = project_id
        self._file_repo = FileRepository(conn)
        self._import_repo = ImportRepository(conn)

    def get_stats(self) -> dict[str, Any]:
        """Get overview statistics for the project."""
        file_stats = self._file_repo.get_file_stats(self.project_id)
        return {
            "total_files": file_stats["total_files"],
            "total_bytes": file_stats["total_bytes"],
            "files_with_sigs": file_stats["files_with_sigs"],
            "total_sigs": file_stats["total_sigs"],
            "imports_count": self._import_repo.get_import_count(self.project_id),
            "edge_count": self._import_repo.get_edge_count(self.project_id),
        }


class ArchitectureProvider:
    """Provides architecture state for a project."""

    def __init__(self, conn: Any, project_id: str):
        self.conn = conn
        self.project_id = project_id
        self._arch_repo = ArchitectureRepository(conn)

    def get_text(self) -> str:
        """Get architecture description text."""
        row = self._arch_repo.get_latest(self.project_id)
        if not row:
            return ""
        summary, layer_map = row
        text = summary or ""
        if layer_map:
            text += f"\n\nCrate Structure:\n```\n{layer_map}\n```"
        return text


class ChangedFilesProvider:
    """Provides changed file detection."""

    def __init__(self, conn: Any, project_id: str):
        self.conn = conn
        self.project_id = project_id
        self._file_repo = FileRepository(conn)

    def get_changed_paths(self) -> list[str]:
        """Get list of changed file paths."""
        changed = self._file_repo.get_changed(self.project_id)
        return [path for _, path in changed]

    def get_blast_radius(self, changed_paths: list[str]) -> set[str]:
        """Get files in the blast radius of changed files."""
        return self._file_repo.get_blast_radius(self.project_id, changed_paths)


class GhostContextProvider:
    """Provides ghost context (unchanged file signatures)."""

    def __init__(self, conn: Any, project_id: str, max_ghost_lines: int = 500):
        self.conn = conn
        self.project_id = project_id
        self.max_ghost_lines = max_ghost_lines
        self._file_repo = FileRepository(conn)

    def get_unchanged_with_sigs(
        self, exclude_paths: list[str] | None = None
    ) -> list[tuple[str, list[Any]]]:
        """Get unchanged files with their signature caches.

        Respects max_ghost_lines limit.
        """
        all_unchanged = self._file_repo.get_unchanged_with_sigs(
            self.project_id, exclude_paths
        )

        if not all_unchanged:
            return []

        # Apply max_ghost_lines limit
        result = []
        line_count = 0
        for fpath, sig_cache in all_unchanged:
            if line_count >= self.max_ghost_lines:
                break
            result.append((fpath, sig_cache))
            # Count approximate lines: one per signature + 1 for file header
            line_count += len(sig_cache) + 1

        return result


class TargetFileProvider:
    """Provides target files (changed + blast radius) with contents."""

    def __init__(self, conn: Any, project_id: str, repo_path: str, file_system: Any = None):
        self.conn = conn
        self.project_id = project_id
        self.repo_path = repo_path
        self.file_system = file_system
        self._changed_provider = ChangedFilesProvider(conn, project_id)

    def get_target_paths(self) -> tuple[list[str], set[str]]:
        """Get target file paths split into changed and blast radius."""
        changed = self._changed_provider.get_changed_paths()
        blast_radius = self._changed_provider.get_blast_radius(changed)
        return changed, blast_radius


class FindingsProvider:
    """Provides historical findings for a project."""

    def __init__(self, conn: Any, project_id: str):
        self.conn = conn
        self.project_id = project_id
        self._finding_repo = FindingRepository(conn)

    def get_open_findings(self, limit: int = 50) -> list[tuple[str, int | None, int | None, str, str, str]]:
        """Get open findings."""
        return self._finding_repo.get_open_findings(self.project_id, limit)


class RiskPatternsProvider:
    """Provides risk pattern detection results for a project."""

    def __init__(self, conn: Any, project_id: str):
        self.conn = conn
        self.project_id = project_id
        self._risk_repo = RiskPatternRepository(conn)

    def get_file_patterns(self) -> list[tuple[str, list[tuple[str, int]]]]:
        """Get risk patterns grouped by file.

        Returns:
            List of (file_path, [(pattern_type, count), ...]) tuples,
            sorted by total risk count descending.
        """
        raw = self._risk_repo.get_project_patterns(self.project_id)
        # Sort by total count per file (descending)
        sorted_files = sorted(
            raw,
            key=lambda x: sum(count for _, count in x[1]),
            reverse=True,
        )
        return sorted_files

    def get_summary(self) -> dict[str, int]:
        """Get total counts per pattern type across the project."""
        return self._risk_repo.get_pattern_summary(self.project_id)


__all__ = [
    "OverviewProvider",
    "ArchitectureProvider",
    "ChangedFilesProvider",
    "GhostContextProvider",
    "TargetFileProvider",
    "FindingsProvider",
    "RiskPatternsProvider",
]
