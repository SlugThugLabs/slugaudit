"""Connection pool management with single-connection-per-call pattern.

Provides a connection pool that is lazily initialized and reused across
tool calls. Handles schema auto-initialization.
"""

import asyncio
import logging
import threading
from contextlib import asynccontextmanager
from typing import Any
from collections.abc import AsyncIterator

from infrastructure import ConnectionPool
from services.schema_service import SchemaService
from app.config import load_config

logger = logging.getLogger("slugaudit-mcp.pool")

_pool: ConnectionPool | None = None
_pool_lock = threading.Lock()
_schema_initialized = False
_schema_lock = asyncio.Lock()
_schema_service = SchemaService()


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
                minconn=1,
                maxconn=5,
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
        try:
            await _run_blocking_safely(pool.putconn, conn)
        except Exception as e:
            logger.debug("Failed to return connection to pool: %s", e)
            try:
                await asyncio.to_thread(conn.close)
            except Exception as e2:
                logger.debug("Failed to close connection: %s", e2)


async def release_connection(conn: Any) -> None:
    """Explicitly release a connection back to the pool."""
    pool = get_pool()
    if pool is not None:
        try:
            await _run_blocking_safely(pool.putconn, conn)
        except Exception as e:
            logger.debug("Failed to return connection to pool: %s", e)
            try:
                await asyncio.to_thread(conn.close)
            except Exception as e2:
                logger.debug("Failed to close connection: %s", e2)
