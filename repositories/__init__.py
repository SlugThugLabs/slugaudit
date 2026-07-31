"""Repository layer — data access abstraction.

Two backends exist: the PostgreSQL repositories below (used when PostgreSQL
is configured — see app/config.py) and their SQLite siblings in
repositories/sqlite/ (the zero-config local fallback). Callers that need to
work with either backend should use the make_*_repository() factories, which
pick the right implementation from the connection's own type — a psycopg2
connection or a sqlite3.Connection — rather than importing a concrete class
directly. Every factory-selected pair implements the same method
names/signatures, so calling code never branches on which backend is active.
"""

import sqlite3
from typing import Any

from .base import BaseRepository, repository_transaction
from .project_repo import ProjectRepository
from .file_repo import FileRepository
from .import_repo import ImportRepository
from .finding_repo import FindingRepository
from .risk_repo import RiskPatternRepository
from .sqlite import (
    SqliteFileRepository,
    SqliteFindingRepository,
    SqliteImportRepository,
    SqliteProjectRepository,
    SqliteRiskPatternRepository,
)


def _is_sqlite(conn: Any) -> bool:
    return isinstance(conn, sqlite3.Connection)


def make_project_repository(conn: Any, *, auto_commit: bool = True) -> Any:
    if _is_sqlite(conn):
        return SqliteProjectRepository(conn, auto_commit=auto_commit)
    return ProjectRepository(conn, auto_commit=auto_commit)


def make_file_repository(conn: Any, *, auto_commit: bool = True) -> Any:
    if _is_sqlite(conn):
        return SqliteFileRepository(conn, auto_commit=auto_commit)
    return FileRepository(conn, auto_commit=auto_commit)


def make_import_repository(conn: Any, *, auto_commit: bool = True) -> Any:
    if _is_sqlite(conn):
        return SqliteImportRepository(conn, auto_commit=auto_commit)
    return ImportRepository(conn, auto_commit=auto_commit)


def make_finding_repository(conn: Any, *, auto_commit: bool = True) -> Any:
    if _is_sqlite(conn):
        return SqliteFindingRepository(conn, auto_commit=auto_commit)
    return FindingRepository(conn, auto_commit=auto_commit)


def make_risk_pattern_repository(conn: Any, *, auto_commit: bool = True) -> Any:
    if _is_sqlite(conn):
        return SqliteRiskPatternRepository(conn, auto_commit=auto_commit)
    return RiskPatternRepository(conn, auto_commit=auto_commit)


__all__ = [
    "BaseRepository",
    "repository_transaction",
    "ProjectRepository",
    "FileRepository",
    "ImportRepository",
    "FindingRepository",
    "RiskPatternRepository",
    "SqliteProjectRepository",
    "SqliteFileRepository",
    "SqliteImportRepository",
    "SqliteFindingRepository",
    "SqliteRiskPatternRepository",
    "make_project_repository",
    "make_file_repository",
    "make_import_repository",
    "make_finding_repository",
    "make_risk_pattern_repository",
]
