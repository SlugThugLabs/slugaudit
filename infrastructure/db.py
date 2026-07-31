"""PostgreSQL connection pooling infrastructure."""

import threading
from typing import Any

import psycopg2
import psycopg2.pool  # required for SimpleConnectionPool (not auto-imported)


class ConnectionPool:
    """Thread-safe connection pool wrapper around psycopg2.

    Provides a persistent pool of connections for long-running processes
    like the MCP server. Connections are automatically returned to the
    pool when closed.

    Usage:
        pool = ConnectionPool(minconn=1, maxconn=5, host=..., dbname=..., user=...)
        conn = pool.getconn()
        # use conn...
        conn.close()  # returns to pool
        pool.closeall()  # at shutdown
    """

    def __init__(
        self,
        minconn: int = 1,
        maxconn: int = 5,
        host: str | None = None,
        port: int = 5432,
        dbname: str | None = None,
        user: str | None = None,
        password: str | None = None,
        options: str | None = None,
    ):
        """Create a connection pool.

        Args:
            minconn: Minimum number of connections to keep in the pool.
            maxconn: Maximum number of connections in the pool.
            options: PostgreSQL connection options (e.g. "-c statement_timeout=30000").
        """
        self.host = host
        self.port = port
        self.dbname = dbname
        self.user = user
        self.password = password
        self.options = options or "-c statement_timeout=30000"
        self._pool = None
        self._minconn = minconn
        self._maxconn = maxconn
        self._creation_lock = threading.Lock()

    @property
    def pool(self) -> Any:
        if self._pool is None:
            with self._creation_lock:
                if self._pool is None:
                    self._pool = self._create_pool()
        return self._pool

    def _create_pool(self) -> Any:
        """Construct the underlying thread-safe psycopg2 pool once."""
        return psycopg2.pool.ThreadedConnectionPool(
            self._minconn, self._maxconn,
            host=self.host,
            port=self.port,
            dbname=self.dbname,
            user=self.user,
            password=self.password or "",
            options=self.options,
        )

    def getconn(self) -> Any:
        """Get a connection from the pool."""
        return self.pool.getconn()

    def putconn(self, conn: Any) -> None:
        """Return a connection to the pool."""
        self.pool.putconn(conn)

    def closeall(self) -> None:
        """Close all connections and destroy the pool."""
        if self._pool is not None:
            self._pool.closeall()
            self._pool = None


__all__ = [
    "ConnectionPool",
]
