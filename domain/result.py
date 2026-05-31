"""Domain — ImportResult and related value objects."""

from dataclasses import dataclass
from typing import Optional


@dataclass
class ImportResult:
    """Result of an import operation."""

    project_id: Optional[str] = None
    project_name: str = ""
    language: str = ""
    files_processed: int = 0
    signatures_extracted: int = 0
    imports_extracted: int = 0
    dependency_edges: int = 0
    elapsed_seconds: float = 0.0

    def __str__(self) -> str:
        return (
            f"Import complete for project: {self.project_name}\n"
            f"  ID: {self.project_id}\n"
            f"  Language: {self.language}\n"
            f"  Files processed: {self.files_processed}\n"
            f"  Signatures extracted: {self.signatures_extracted}\n"
            f"  Imports extracted: {self.imports_extracted}\n"
            f"  Dependency edges: {self.dependency_edges}\n"
            f"  Elapsed: {self.elapsed_seconds:.1f}s"
        )


__all__ = ["ImportResult"]
