"""Deterministic discovery and hashing of a project's supported source files."""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

from languages import language_for_path, supported_extensions


PARSER_VERSION = "treesitter-v1"

_SKIP_DIRS = frozenset(
    {
        ".git",
        ".planning",
        ".claude",
        ".codex",
        ".venv",
        ".next",
        ".nuxt",
        "__pycache__",
        "build",
        "dist",
        "node_modules",
        "target",
        "vendor",
        "venv",
    }
)


@dataclass(frozen=True)
class SourceFile:
    """One source file in the on-disk manifest."""

    path: str
    hash: str
    size: int
    language: str

    def to_dict(self) -> dict[str, str | int]:
        return {
            "hash": self.hash,
            "size": self.size,
            "language": self.language,
        }


@dataclass(frozen=True)
class SourceManifest:
    """Complete supported source set for one project root."""

    files: dict[str, SourceFile]
    manifest_hash: str
    languages: tuple[str, ...]

    @property
    def file_count(self) -> int:
        return len(self.files)


def _git_paths(project_root: Path) -> list[str] | None:
    """Return tracked and untracked non-ignored paths, or None outside Git."""
    git_executable = shutil.which("git")
    if git_executable is None:
        return None
    try:
        result = subprocess.run(  # noqa: S603 - executable resolved from PATH
            [
                git_executable,
                "-C",
                str(project_root),
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
            check=False,
            capture_output=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None

    if result.returncode != 0:
        return None
    return sorted(
        path.decode("utf-8", errors="surrogateescape")
        for path in result.stdout.split(b"\0")
        if path
    )


def _walk_paths(project_root: Path) -> list[str]:
    """Discover source candidates for non-Git projects."""
    paths: list[str] = []
    for dirpath, dirnames, filenames in os.walk(project_root, followlinks=False):
        dirnames[:] = sorted(
            name
            for name in dirnames
            if name not in _SKIP_DIRS
            and not name.startswith(".")
            and not (Path(dirpath) / name).is_symlink()
        )
        for filename in sorted(filenames):
            full_path = Path(dirpath) / filename
            if full_path.is_symlink():
                continue
            paths.append(full_path.relative_to(project_root).as_posix())
    return paths


def discover_source_paths(project_root: str | Path) -> list[str]:
    """Return every supported, non-ignored source path in deterministic order."""
    root = Path(project_root).resolve()
    extensions = supported_extensions()
    candidates = _git_paths(root)
    if candidates is None:
        candidates = _walk_paths(root)

    result: list[str] = []
    for relpath in candidates:
        path = Path(relpath)
        if not path.parts or any(part in _SKIP_DIRS for part in path.parts):
            continue
        if path.suffix.lower() not in extensions:
            continue
        full_path = root / path
        if not full_path.is_file() or full_path.is_symlink():
            continue
        result.append(path.as_posix())
    return sorted(set(result))


def _hash_file(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
    return digest.hexdigest(), size


def _hash_manifest(entries: Iterable[SourceFile]) -> str:
    digest = hashlib.sha256()
    for entry in entries:
        digest.update(entry.path.encode("utf-8", errors="surrogateescape"))
        digest.update(b"\0")
        digest.update(entry.hash.encode("ascii"))
        digest.update(b"\0")
        digest.update(entry.language.encode("ascii"))
        digest.update(b"\0")
    digest.update(PARSER_VERSION.encode("ascii"))
    return digest.hexdigest()


def build_manifest(project_root: str | Path) -> SourceManifest:
    """Read and hash the complete supported source set."""
    root = Path(project_root).resolve()
    files: dict[str, SourceFile] = {}
    for relpath in discover_source_paths(root):
        digest, size = _hash_file(root / relpath)
        language = language_for_path(relpath)
        if language is None:
            continue
        files[relpath] = SourceFile(
            path=relpath,
            hash=digest,
            size=size,
            language=language,
        )

    entries = [files[path] for path in sorted(files)]
    return SourceManifest(
        files=files,
        manifest_hash=_hash_manifest(entries),
        languages=tuple(sorted({entry.language for entry in entries})),
    )


__all__ = [
    "PARSER_VERSION",
    "SourceFile",
    "SourceManifest",
    "build_manifest",
    "discover_source_paths",
]
