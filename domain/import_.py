"""Domain entities — Import records and dependency edges."""

from typing import Any
from dataclasses import dataclass


@dataclass
class ImportRecord:
    """An import/use/include statement in a source file."""
    import_text: str
    import_type: str  # "internal" or "external"
    line_start: int | None = None
    line_end: int | None = None
    resolved_path: str | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "ImportRecord":
        return cls(
            import_text=d["import_text"],
            import_type=d.get("import_type", "internal"),
            line_start=d.get("line_start"),
            line_end=d.get("line_end"),
            resolved_path=d.get("resolved_path"),
        )


@dataclass
class DependencyEdge:
    """A dependency edge between two files."""
    project_id: str
    source_file_id: str
    target_file_id: str
    import_id: int | None = None


__all__ = ["ImportRecord", "DependencyEdge"]
