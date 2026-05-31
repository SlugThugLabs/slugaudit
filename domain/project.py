"""Domain entities — Project, File, Signature."""

from dataclasses import dataclass, field
from typing import Optional
from datetime import datetime


@dataclass
class Project:
    """A project in the audit database."""
    id: str
    name: str
    primary_language: str
    repo_path: str
    created_at: Optional[datetime] = None
    updated_at: Optional[datetime] = None
    config_id: Optional[str] = None


@dataclass
class File:
    """A source file in a project."""
    id: str
    project_id: str
    path: str
    hash: str
    size: int
    last_modified_at: Optional[str] = None
    last_audited_at: Optional[datetime] = None
    last_audited_hash: Optional[str] = None
    signature_cache: Optional[list] = None
    created_at: Optional[datetime] = None
    updated_at: Optional[datetime] = None


@dataclass
class Signature:
    """A code signature extracted from a source file."""
    sig_type: str  # fn, struct, class, enum, trait, impl, const, type_alias, macro
    name: str
    signature: str
    visibility: str = ""
    doc_comment: str = ""
    line_start: Optional[int] = None
    line_end: Optional[int] = None
    is_async: bool = False
    is_unsafe: bool = False
    generic_params: Optional[str] = None

    def to_cache_dict(self) -> dict:
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
