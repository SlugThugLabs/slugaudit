"""Versioned project state under ``.planning/slugaudit``."""

from __future__ import annotations

import json
import logging
import os
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from app.manifest import PARSER_VERSION, SourceManifest


logger = logging.getLogger("slugaudit-mcp.state")

CONTRACT_VERSION = 1
SCHEMA_VERSION = 4
STATE_FILENAME = "state.json"


@dataclass
class ProjectState:
    """Last atomically published database revision for a project."""

    project_path: str = ""
    project_name: str = ""
    project_id: str = ""
    revision_id: str = ""
    manifest_hash: str = ""
    last_synced_at: str = ""
    file_count: int = 0
    signature_count: int = 0
    languages: list[str] = field(default_factory=list)
    files: dict[str, dict[str, str | int]] = field(default_factory=dict)
    contract_version: int = CONTRACT_VERSION
    schema_version: int = SCHEMA_VERSION
    parser_version: str = PARSER_VERSION

    @property
    def language(self) -> str:
        """Compatibility summary used by existing handlers and ClauRust."""
        if not self.languages:
            return "unknown"
        if len(self.languages) == 1:
            return self.languages[0]
        return "polyglot"

    def to_dict(self) -> dict[str, Any]:
        """Serialize using the shared snake_case adapter contract."""
        file_signatures = {
            path: str(metadata["hash"])
            for path, metadata in sorted(self.files.items())
        }
        return {
            "contract_version": self.contract_version,
            "schema_version": self.schema_version,
            "project_path": self.project_path,
            "project_name": self.project_name,
            "project_id": self.project_id,
            "revision_id": self.revision_id,
            "manifest_hash": self.manifest_hash,
            "parser_version": self.parser_version,
            "last_synced_at": self.last_synced_at,
            "file_count": self.file_count,
            "signature_count": self.signature_count,
            "language": self.language,
            "languages": sorted(self.languages),
            "files": {path: self.files[path] for path in sorted(self.files)},
            # Transitional compatibility for the existing ClauRust reader.
            "file_signatures": file_signatures,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> ProjectState:
        """Deserialize only the current, complete state contract."""
        required = {
            "contract_version",
            "schema_version",
            "project_path",
            "project_name",
            "project_id",
            "revision_id",
            "manifest_hash",
            "parser_version",
            "last_synced_at",
            "file_count",
            "signature_count",
            "files",
        }
        if not required.issubset(data):
            missing = ", ".join(sorted(required.difference(data)))
            raise ValueError(f"state is missing required fields: {missing}")
        if data["contract_version"] != CONTRACT_VERSION:
            raise ValueError(
                f"unsupported contract version: {data['contract_version']}"
            )
        if data["schema_version"] != SCHEMA_VERSION:
            raise ValueError(f"unsupported schema version: {data['schema_version']}")
        if data["parser_version"] != PARSER_VERSION:
            raise ValueError(f"unsupported parser version: {data['parser_version']}")
        if not isinstance(data["files"], dict):
            raise ValueError("state files must be an object")

        files: dict[str, dict[str, str | int]] = {}
        for path, metadata in data["files"].items():
            if not isinstance(path, str) or not isinstance(metadata, dict):
                raise ValueError("invalid file manifest entry")
            if not {"hash", "size", "language"}.issubset(metadata):
                raise ValueError(f"incomplete file manifest entry: {path}")
            files[path] = {
                "hash": str(metadata["hash"]),
                "size": int(metadata["size"]),
                "language": str(metadata["language"]),
            }

        languages = data.get("languages")
        if not isinstance(languages, list):
            legacy_language = data.get("language")
            languages = [legacy_language] if isinstance(legacy_language, str) else []

        state = cls(
            project_path=str(data["project_path"]),
            project_name=str(data["project_name"]),
            project_id=str(data["project_id"]),
            revision_id=str(data["revision_id"]),
            manifest_hash=str(data["manifest_hash"]),
            last_synced_at=str(data["last_synced_at"]),
            file_count=int(data["file_count"]),
            signature_count=int(data["signature_count"]),
            languages=[str(item) for item in languages],
            files=files,
        )
        if not all(
            (
                state.project_path,
                state.project_name,
                state.project_id,
                state.revision_id,
                state.manifest_hash,
                state.last_synced_at,
            )
        ):
            raise ValueError("state identity and revision fields cannot be empty")
        if state.file_count != len(files):
            raise ValueError("state file count does not match its manifest")
        return state

    @classmethod
    def from_sync_result(
        cls,
        project_path: str,
        project_id: str,
        revision_id: str,
        manifest: SourceManifest,
        signature_count: int,
        synced_at: str,
    ) -> ProjectState:
        root = str(Path(project_path).resolve())
        return cls(
            project_path=root,
            project_name=Path(root).name,
            project_id=project_id,
            revision_id=revision_id,
            manifest_hash=manifest.manifest_hash,
            last_synced_at=synced_at,
            file_count=manifest.file_count,
            signature_count=signature_count,
            languages=list(manifest.languages),
            files={path: entry.to_dict() for path, entry in manifest.files.items()},
        )


def state_dir(project_root: str | Path) -> Path:
    """Return the activation directory for a project root."""
    return Path(project_root).resolve() / ".planning" / "slugaudit"


def _validated_state_dir(project_root: str | Path) -> Path:
    """Return the activation path only when no state component is a symlink."""
    root = Path(project_root).resolve()
    planning = root / ".planning"
    activation = planning / "slugaudit"
    if planning.is_symlink() or activation.is_symlink():
        raise RuntimeError("Refusing a symlinked SlugAudit activation path")
    return activation


def find_project_root(start: str | Path) -> Path:
    """Find the nearest parent whose activation directory exists."""
    current = Path(start).resolve()
    if current.is_file():
        current = current.parent
    for candidate in (current, *current.parents):
        activation = _validated_state_dir(candidate)
        if activation.is_dir():
            return candidate
    raise RuntimeError(
        "SlugAudit is not enabled for this project. "
        "Create .planning/slugaudit with `/slugaudit on`."
    )


def load_state(project_root: str | Path) -> ProjectState | None:
    """Load valid state; invalid or old state requests a safe full rebuild."""
    state_file = _validated_state_dir(project_root) / STATE_FILENAME
    if not state_file.is_file():
        return None
    try:
        data = json.loads(state_file.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            raise ValueError("state root must be an object")
        state = ProjectState.from_dict(data)
        expected_path = str(Path(project_root).resolve())
        if state.project_path != expected_path:
            raise ValueError("state belongs to a different project path")
        return state
    except (json.JSONDecodeError, OSError, TypeError, ValueError) as error:
        logger.warning("Ignoring invalid SlugAudit state: %s", error)
        return None


def save_state(project_root: str | Path, state: ProjectState) -> None:
    """Atomically replace state without ever creating the activation trigger."""
    directory = _validated_state_dir(project_root)
    if not directory.is_dir():
        raise RuntimeError(
            "SlugAudit was disabled while syncing; refusing to recreate its trigger"
        )

    payload = json.dumps(state.to_dict(), indent=2, sort_keys=True) + "\n"
    fd, temporary_name = tempfile.mkstemp(
        prefix=".state-", suffix=".tmp", dir=directory
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as temporary:
            temporary.write(payload)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_name, directory / STATE_FILENAME)
        directory_fd = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    except Exception:
        try:
            os.unlink(temporary_name)
        except FileNotFoundError:
            pass
        raise

__all__ = [
    "CONTRACT_VERSION",
    "SCHEMA_VERSION",
    "ProjectState",
    "find_project_root",
    "load_state",
    "save_state",
    "state_dir",
]
