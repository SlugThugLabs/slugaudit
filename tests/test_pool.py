"""Concurrency regressions for database pool orchestration."""

# ruff: noqa: S101 - pytest assertions provide the clearest contract failures.

import asyncio
import threading
import time
from unittest.mock import MagicMock, patch

import pytest

from app import pool as pool_module


def test_concurrent_schema_checks_do_not_deadlock_event_loop() -> None:
    calls = 0

    def initialize(_conn: object, _logger: object) -> None:
        nonlocal calls
        calls += 1
        time.sleep(0.02)

    async def run() -> None:
        await asyncio.wait_for(
            asyncio.gather(
                pool_module._ensure_schema(object()),
                pool_module._ensure_schema(object()),
            ),
            timeout=1,
        )

    with (
        patch.object(pool_module, "_schema_initialized", False),
        patch.object(pool_module, "_schema_lock", asyncio.Lock()),
        patch.object(pool_module._schema_service, "initialize", side_effect=initialize),
    ):
        asyncio.run(run())

    assert calls == 1


def test_cancelled_thread_worker_finishes_before_connection_can_be_reused() -> None:
    started = threading.Event()
    release = threading.Event()
    finished = threading.Event()

    def worker() -> str:
        started.set()
        release.wait(timeout=1)
        finished.set()
        return "done"

    async def run() -> None:
        task = asyncio.create_task(pool_module._run_blocking_safely(worker))
        while not started.is_set():
            await asyncio.sleep(0)
        task.cancel()
        asyncio.get_running_loop().call_later(0.02, release.set)
        with pytest.raises(asyncio.CancelledError):
            await task
        assert finished.is_set()

    asyncio.run(run())


def test_configured_pool_uses_thread_safe_psycopg_pool() -> None:
    threaded_pool = MagicMock()
    with patch(
        "infrastructure.db.psycopg2.pool.ThreadedConnectionPool",
        return_value=threaded_pool,
    ) as constructor:
        pool = pool_module.ConnectionPool(
            minconn=1,
            maxconn=2,
            host="db.example",
            dbname="audit",
            user="agent",
        )

        assert pool.pool is threaded_pool
        assert pool.pool is threaded_pool

    constructor.assert_called_once()


def test_failed_call_rolls_back_before_connection_is_reused() -> None:
    events: list[str] = []
    connection = MagicMock()
    connection.rollback.side_effect = lambda: events.append("rollback")
    pool = MagicMock()
    pool.getconn.return_value = connection
    pool.putconn.side_effect = lambda _connection: events.append("putconn")

    async def run() -> None:
        with (
            patch.object(pool_module, "get_pool", return_value=pool),
            patch.object(pool_module, "_ensure_schema", return_value=None),
        ):
            with pytest.raises(RuntimeError, match="query failed"):
                async with pool_module.get_connection():
                    raise RuntimeError("query failed")

    asyncio.run(run())

    connection.rollback.assert_called_once_with()
    pool.putconn.assert_called_once_with(connection)
    assert events == ["rollback", "putconn"]


def test_connection_is_discarded_when_cleanup_rollback_fails() -> None:
    connection = MagicMock()
    connection.rollback.side_effect = RuntimeError("transaction is broken")
    pool = MagicMock()

    asyncio.run(pool_module._return_connection(pool, connection))

    connection.close.assert_called_once_with()
    pool.putconn.assert_not_called()
