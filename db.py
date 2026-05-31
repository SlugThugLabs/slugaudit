"""PostgreSQL connection and upsert helpers for audit database.

This module re-exports from the new package structure for backward
compatibility. New code should import from infrastructure/ and
repositories/ directly.
"""

import json

# Re-export from infrastructure for backward compatibility
from infrastructure.db import (
    parse_connection_string,
    get_connection,
    ConnectionPool,
)

# Re-export psycopg2 for backward-compatible patching in tests
import psycopg2

# Legacy functions — delegate to repositories for backward compatibility
# New code should use repositories directly.


def get_or_create_project(conn, name: str, language: str, repo_path: str) -> str:
    """Get existing project or create a new one. Returns project UUID."""
    from repositories import ProjectRepository
    repo = ProjectRepository(conn)
    return repo.get_or_create(name, language, repo_path)


def upsert_file(
    conn, project_id: str, relpath: str, file_hash: str, file_size: int,
    mtime, signatures: list, force: bool = False,
) -> tuple:
    """Upsert a file record. Returns (file_id, was_updated)."""
    from repositories import FileRepository
    repo = FileRepository(conn)
    return repo.upsert(project_id, relpath, file_hash, file_size, mtime, signatures, force)


def delete_removed_files(conn, project_id: str, active_paths: set):
    """Remove files from DB that no longer exist on disk."""
    from repositories import FileRepository
    repo = FileRepository(conn)
    repo.delete_removed(project_id, active_paths)


def insert_imports(conn, project_id: str, file_id: str, imports: list[dict], force: bool = False):
    """Insert import records for a file. Clears old ones if force."""
    from repositories import ImportRepository
    repo = ImportRepository(conn)
    repo.insert(project_id, file_id, imports, force)


def build_dependency_edges(conn, project_id: str, importer, force: bool = False):
    """Resolve all file_imports to dependency edges between files."""
    from repositories import ImportRepository
    repo = ImportRepository(conn)
    return repo.build_dependency_edges(project_id, importer, force)


def get_changed_files(conn, project_id: str) -> list[tuple]:
    """Return (id, path) for files where hash differs from last_audited_hash."""
    from repositories import FileRepository
    repo = FileRepository(conn)
    return repo.get_changed(project_id)


def get_project_names(conn) -> list[str]:
    """Return all project names."""
    from repositories import ProjectRepository
    repo = ProjectRepository(conn)
    return repo.get_names()


def schema_exists(conn) -> bool:
    """Check if the slugaudit-mcp schema has been initialized."""
    from repositories import ProjectRepository
    repo = ProjectRepository(conn)
    cur = repo._cursor()
    try:
        cur.execute(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables "
            "WHERE table_name = 'projects')"
        )
        result = cur.fetchone()[0]
        cur.close()
        return result
    except Exception:
        cur.close()
        return False


def update_audit_timestamps(conn, project_id: str):
    """Update last_audited_hash to match current hash for all files."""
    from repositories import FileRepository
    repo = FileRepository(conn)
    repo.update_audit_timestamps(project_id)


__all__ = [
    # infrastructure re-exports
    "parse_connection_string",
    "get_connection",
    "ConnectionPool",
    "psycopg2",
    "json",
    # legacy functions (delegate to repositories)
    "get_or_create_project",
    "upsert_file",
    "delete_removed_files",
    "insert_imports",
    "build_dependency_edges",
    "get_changed_files",
    "get_project_names",
    "schema_exists",
    "update_audit_timestamps",
]
