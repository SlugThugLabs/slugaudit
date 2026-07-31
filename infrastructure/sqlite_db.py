"""SQLite connection handling for the zero-config local backend.

Used only when no PostgreSQL is configured (see app/config.py). One database
file per project, at ``.planning/slugaudit/audit.db`` — there is no shared
server to partition by project_id the way the PostgreSQL backend does, so
each project's evidence lives in its own file instead.

A fresh connection is opened and closed per tool call rather than pooled:
SQLite has no network round-trip to amortize, the existing per-project
``fcntl`` lock (app/sync.py) already serializes cross-process access, and a
connection pool would only add complexity with no real benefit here.
"""

import sqlite3
from pathlib import Path

import regex

# Mirrors the per-line regex timeout in app/handlers.py:handle_search. The
# pattern itself is already validated (length cap, dangerous-quantifier
# heuristic) before it ever reaches SQL — this is the same last-resort bound
# on a single evaluation, not a substitute for that validation.
_REGEXP_TIMEOUT_SECONDS = 0.05


def sqlite_db_path(project_root: str | Path) -> Path:
    """Return the per-project SQLite database path.

    Lives inside the same ``.planning/slugaudit/`` activation directory as
    ``state.json`` and ``sync.lock``, so ``/slugaudit off`` (which removes
    that whole directory) purges it automatically with no special-casing.
    """
    return Path(project_root).resolve() / ".planning" / "slugaudit" / "audit.db"


def _regexp(pattern: str, text: str | None) -> bool:
    """Backs the SQL-level REGEXP operator used by SqliteFileRepository."""
    if text is None:
        return False
    try:
        return regex.search(pattern, text, timeout=_REGEXP_TIMEOUT_SECONDS) is not None
    except (regex.error, TimeoutError):
        return False


def connect(project_root: str | Path) -> sqlite3.Connection:
    """Open a connection to this project's SQLite database.

    The parent ``.planning/slugaudit/`` directory must already exist (it's
    the activation trigger created by ``/slugaudit on``); this never creates
    it, matching PostgreSQL's ``get_connection()`` never creating a project.
    """
    path = sqlite_db_path(project_root)
    conn = sqlite3.connect(str(path), check_same_thread=False)
    conn.execute("PRAGMA foreign_keys = ON")
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA busy_timeout = 3000")
    conn.create_function("REGEXP", 2, _regexp)
    return conn


__all__ = ["connect", "sqlite_db_path"]
