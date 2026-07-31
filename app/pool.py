"""Connection pool management with single-connection-per-call pattern.

Provides a connection pool that is lazily initialized and reused across
tool calls. Handles schema auto-initialization.

Two backends: PostgreSQL (this module's original get_connection(), used when
PGHOST/PGDATABASE/PGUSER or config.toml fully resolve — app.config.Config.
is_configured) and a zero-config SQLite fallback, one file per project at
.planning/slugaudit/audit.db, used otherwise. get_connection_for_project()
is what callers should use — it picks the right backend per call.
"""

import asyncio
import logging
import threading
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any
from collections.abc import AsyncIterator

from infrastructure import ConnectionPool
from infrastructure.sqlite_db import connect as sqlite_connect
from services.schema_service import SchemaService
from services.sqlite_schema_service import SqliteSchemaService
from app.config import load_config

logger = logging.getLogger("slugaudit-mcp.pool")

_pool: ConnectionPool | None = None
_pool_lock = threading.Lock()
_schema_initialized = False
_schema_lock = asyncio.Lock()
_schema_service = SchemaService()
_sqlite_schema_service = SqliteSchemaService()


async def _run_blocking_safely(function: Any, *args: Any) -> Any:
    """Finish a connection-owning worker before propagating cancellation.

    ``asyncio.to_thread`` cannot stop an in-flight DB call. Returning its
    connection to the pool while that worker still uses it corrupts concurrent
    requests, so cancellation waits for the worker to finish first.
    """
    task = asyncio.create_task(asyncio.to_thread(function, *args))
    try:
        return await asyncio.shield(task)
    except asyncio.CancelledError:
        try:
            await task
        finally:
            raise


def init_pool() -> None:
    """Initialize the connection pool from config. Idempotent."""
    global _pool
    with _pool_lock:
        if _pool is not None:
            return
        cfg = load_config()
        if not cfg.is_configured:
            logger.warning("Database not configured")
            return
        try:
            _pool = ConnectionPool(
                minconn=cfg.pool_min,
                maxconn=cfg.pool_max,
                host=cfg.host,
                port=cfg.port,
                dbname=cfg.database,
                user=cfg.user,
                password=cfg.password,
            )
        except Exception as e:
            logger.warning(f"Could not initialize connection pool: {e}")
            _pool = None


def get_pool() -> ConnectionPool | None:
    """Get the connection pool, initializing if needed."""
    if _pool is None:
        init_pool()
    return _pool


async def _ensure_schema(conn: Any) -> None:
    """Initialize DB schema if not already done. Thread-safe."""
    global _schema_initialized
    if _schema_initialized:
        return
    async with _schema_lock:
        if _schema_initialized:
            return
        # Always run all idempotent migrations. One legacy table existing does
        # not prove that the rest of the required schema is current.
        await _run_blocking_safely(_schema_service.initialize, conn, logger)
        _schema_initialized = True


async def _return_connection(pool: ConnectionPool, conn: Any) -> None:
    """Return only a transaction-clean connection to the shared pool."""
    try:
        await _run_blocking_safely(conn.rollback)
    except Exception as error:
        logger.warning(
            "Discarding database connection after rollback failed: %s",
            error,
        )
        try:
            await asyncio.to_thread(conn.close)
        except Exception as close_error:
            logger.debug("Failed to close poisoned connection: %s", close_error)
        return

    try:
        await _run_blocking_safely(pool.putconn, conn)
    except Exception as error:
        logger.debug("Failed to return connection to pool: %s", error)
        try:
            await asyncio.to_thread(conn.close)
        except Exception as close_error:
            logger.debug("Failed to close connection: %s", close_error)


@asynccontextmanager
async def get_connection() -> AsyncIterator[Any]:
    """Get a database connection from the pool.

    Yields a connection that is automatically returned to the pool.
    Initializes the schema on first use.

    Usage:
        async with get_connection() as conn:
            cur = conn.cursor()
            ...
    """
    pool = get_pool()
    if pool is None:
        raise RuntimeError(
            "Database not configured. Set PGHOST, PGDATABASE, PGUSER, PGPASSWORD "
            "or create a config.toml file."
        )

    conn = await _run_blocking_safely(pool.getconn)
    try:
        await _ensure_schema(conn)
        yield conn
    finally:
        await _return_connection(pool, conn)


async def release_connection(conn: Any) -> None:
    """Explicitly release a connection back to the pool."""
    pool = get_pool()
    if pool is not None:
        await _return_connection(pool, conn)


@asynccontextmanager
async def _get_sqlite_connection(project_root: str) -> AsyncIterator[Any]:
    """Open this project's SQLite database, closing it when the call ends.

    No pool: SQLite has no network round-trip to amortize, and the
    per-project fcntl lock (app/sync.py) already serializes cross-process
    access, so a fresh connection per call is simpler and just as safe.
    """
    activation = Path(project_root).resolve() / ".planning" / "slugaudit"
    if not activation.is_dir():
        # Mirrors app.state.find_project_root's message: connecting to a
        # per-project SQLite file only makes sense once that project is
        # activated, and failing here (rather than a raw "unable to open
        # database file") keeps that failure mode identical across backends.
        raise RuntimeError(
            "SlugAudit is not enabled for this project. "
            "Create .planning/slugaudit with `/slugaudit on`."
        )

    conn = await asyncio.to_thread(sqlite_connect, project_root)
    try:
        await asyncio.to_thread(_sqlite_schema_service.initialize, conn, logger)
        yield conn
    finally:
        await asyncio.to_thread(conn.close)


@asynccontextmanager
async def get_connection_for_project(project_root: str) -> AsyncIterator[Any]:
    """Get a connection for whichever backend is configured.

    PostgreSQL when Config.is_configured (env vars or config.toml resolve),
    otherwise the zero-config per-project SQLite fallback. Every caller that
    needs a database connection should go through this rather than
    get_connection() directly, so it works under either backend without
    knowing which one is active.
    """
    if load_config().is_configured:
        async with get_connection() as conn:
            yield conn
        return

    async with _get_sqlite_connection(project_root) as conn:
        yield conn
