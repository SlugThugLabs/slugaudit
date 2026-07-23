"""Domain entities — Project, File, Signature."""

from typing import Any
from dataclasses import dataclass
from datetime import datetime


@dataclass
class Project:
    """A project in the audit database."""
    id: str
    name: str
    primary_language: str
    repo_path: str
    created_at: datetime | None = None
    updated_at: datetime | None = None
    config_id: str | None = None


@dataclass
class File:
    """A source file in a project."""
    id: str
    project_id: str
    path: str
    hash: str
    size: int
    last_modified_at: str | None = None
    last_audited_at: datetime | None = None
    last_audited_hash: str | None = None
    signature_cache: list[Any] | None = None
    created_at: datetime | None = None
    updated_at: datetime | None = None


@dataclass
class Signature:
    """A code signature extracted from a source file."""
    sig_type: str  # fn, struct, class, enum, trait, impl, const, type_alias, macro
    name: str
    signature: str
    visibility: str = ""
    doc_comment: str = ""
    line_start: int | None = None
    line_end: int | None = None
    is_async: bool = False
    is_unsafe: bool = False
    generic_params: str | None = None

    def to_cache_dict(self) -> dict[str, Any]:
        """Convert to JSON-cacheable dict format."""
        entry = {
            "type": self.sig_type,
            "name": self.name,
            "visibility": self.visibility,
            "signature": self.signature,
        }
        if self.generic_params:
            entry["generic_params"] = self.generic_params
        return entry


__all__ = ["Project", "File", "Signature"]
