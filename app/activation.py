"""Host-adapter API for the sole human control: ``/slugaudit on|off``."""

from __future__ import annotations

import fcntl
import shutil
from pathlib import Path
from typing import Any

from app.state import state_dir
from repositories import ProjectRepository


def enable_project(project_root: str | Path) -> Path:
    """Create only the activation directory; first AI query performs import."""
    root = Path(project_root).resolve()
    if not root.is_dir():
        raise ValueError(f"Not a project directory: {root}")
    planning = root / ".planning"
    if planning.is_symlink():
        raise ValueError("Refusing a symlinked .planning directory")
    activation = state_dir(root)
    if activation.exists() and activation.is_symlink():
        raise ValueError("Refusing a symlinked .planning/slugaudit directory")
    activation.mkdir(parents=True, exist_ok=True)
    return activation


def disable_project(project_root: str | Path, conn: Any) -> bool:
    """Purge project-owned DB evidence, then remove the activation directory.

    The directory is retained when database cleanup fails, so a host cannot
    report a successful ``off`` while stale project data remains stored.
    """
    root = Path(project_root).resolve()
    if (root / ".planning").is_symlink():
        raise ValueError("Refusing a symlinked .planning directory")
    activation = state_dir(root)
    if not activation.exists():
        return False
    if activation.is_symlink() or not activation.is_dir():
        raise ValueError("Refusing to remove an invalid SlugAudit activation path")

    lock_path = activation / "sync.lock"
    with lock_path.open("a+", encoding="utf-8") as lock_file:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
        try:
            ProjectRepository(conn).purge_by_path(str(root))
            shutil.rmtree(activation)
        finally:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
    return True


__all__ = ["disable_project", "enable_project"]
