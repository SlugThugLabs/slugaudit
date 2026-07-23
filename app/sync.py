"""Mandatory filesystem freshness gate executed before every MCP query."""

from __future__ import annotations

import asyncio
import fcntl
import logging
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, TextIO

from app.manifest import PARSER_VERSION, SourceManifest, build_manifest
from app.state import ProjectState, find_project_root, load_state, save_state, state_dir
from repositories import FileRepository, ProjectRepository
from services.import_service import reconcile_project


logger = logging.getLogger("slugaudit-mcp.sync")


@asynccontextmanager
async def _project_lock(project_root: Path) -> AsyncIterator[None]:
    """Serialize sync across MCP processes sharing the activation directory."""
    lock_path = state_dir(project_root) / "sync.lock"
    try:
        lock_file: TextIO = lock_path.open("a+", encoding="utf-8")
    except OSError as error:
        raise RuntimeError(f"Cannot open SlugAudit sync lock: {error}") from error
    try:
        await asyncio.to_thread(fcntl.flock, lock_file.fileno(), fcntl.LOCK_EX)
        yield
    finally:
        try:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        finally:
            lock_file.close()


def _state_hashes(state: ProjectState) -> dict[str, str]:
    return {
        path: str(metadata["hash"])
        for path, metadata in state.files.items()
    }


def _manifest_hashes(manifest: SourceManifest) -> dict[str, str]:
    return {path: entry.hash for path, entry in manifest.files.items()}


def _database_matches_state(
    conn: Any,
    project_root: Path,
    state: ProjectState,
) -> bool:
    """Prove that local state identifies the database rows tools will query."""
    project_repo = ProjectRepository(conn)
    project = project_repo.get_by_path(str(project_root))
    if project is None or str(project[0]) != state.project_id:
        return False

    revision = project_repo.get_current_revision(state.project_id)
    if revision is None:
        return False
    if str(revision["revision_id"]) != state.revision_id:
        return False
    if revision["manifest_hash"] != state.manifest_hash:
        return False
    if revision["parser_version"] != PARSER_VERSION:
        return False
    if int(revision["file_count"]) != state.file_count:
        return False
    if int(revision["signature_count"]) != state.signature_count:
        return False

    return bool(
        FileRepository(conn).get_manifest(state.project_id) == _state_hashes(state)
    )


def _sync_locked(project_root: Path, conn: Any) -> ProjectState:
    """Verify disk, local state, and DB before returning a usable revision."""
    if not state_dir(project_root).is_dir():
        raise RuntimeError("SlugAudit was disabled before synchronization began")

    manifest = build_manifest(project_root)
    state = load_state(project_root)
    database_matches = (
        state is not None and _database_matches_state(conn, project_root, state)
    )
    disk_matches = (
        state is not None
        and state.manifest_hash == manifest.manifest_hash
        and _state_hashes(state) == _manifest_hashes(manifest)
        and state.parser_version == PARSER_VERSION
    )

    if state is not None and database_matches and disk_matches:
        # Close the implicit read transaction before the handler starts. This
        # is required for audit_raw_sql to open a database-enforced READ ONLY
        # transaction and avoids returning idle transactions to the pool.
        conn.rollback()
        return state

    force_full = state is None or not database_matches
    reason = (
        "state missing or invalid"
        if state is None
        else "database revision mismatch"
        if not database_matches
        else "filesystem manifest changed"
    )
    logger.info("Synchronizing %s (%s)", project_root, reason)

    result = reconcile_project(
        str(project_root),
        manifest,
        conn=conn,
        force_full=force_full,
    )
    if not result.project_id or not result.revision_id:
        raise RuntimeError("SlugAudit import did not publish a database revision")

    synced_at = datetime.now(UTC).isoformat()
    new_state = ProjectState.from_sync_result(
        project_path=str(project_root),
        project_id=str(result.project_id),
        revision_id=result.revision_id,
        manifest=manifest,
        signature_count=result.signatures_extracted,
        synced_at=synced_at,
    )
    save_state(project_root, new_state)
    return new_state


async def ensure_synced(cwd: str, conn: Any | None = None) -> ProjectState:
    """Return only after a complete, current revision is proven and published.

    There is intentionally no stale fallback. A discovery, parsing, database,
    or state-write failure aborts the tool call so an AI cannot reason from
    evidence that SlugAudit failed to verify.
    """
    if conn is not None:
        async with synchronized_project(cwd, conn) as state:
            return state

    from app.pool import get_connection

    async with get_connection() as db_conn:
        async with synchronized_project(cwd, db_conn) as state:
            return state


@asynccontextmanager
async def synchronized_project(
    cwd: str, conn: Any
) -> AsyncIterator[ProjectState]:
    """Hold one project revision stable across sync and the consuming query."""
    project_root = find_project_root(cwd)
    async with _project_lock(project_root):
        try:
            state = await asyncio.to_thread(_sync_locked, project_root, conn)
            yield state
        except Exception as error:
            logger.error("Mandatory auto-sync or query failed: %s", error)
            raise RuntimeError(f"SlugAudit freshness check failed: {error}") from error
        finally:
            # Read handlers leave an implicit transaction open. End it before
            # releasing the project lock so the response cannot outlive the
            # revision it claims to represent.
            try:
                conn.rollback()
            except Exception:
                logger.exception("Failed to close SlugAudit query transaction")


__all__ = ["ensure_synced", "synchronized_project"]
