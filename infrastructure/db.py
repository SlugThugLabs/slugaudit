"""PostgreSQL connection and pooling infrastructure.

Provides connection management with support for connection strings,
environment variables, connection pooling, and credential-safe
error handling.
"""

import os
import re
import threading
from typing import Any

import psycopg2
import psycopg2.pool  # required for SimpleConnectionPool (not auto-imported)


def parse_connection_string(s: str) -> dict[str, Any]:
    """Parse postgresql://user:pass@host:port/dbname into a dict.

    Supports optional query parameters like ?sslmode=require.

    Security: The returned dict contains the password in plaintext but
    error messages never include it.
    """
    m = re.match(
        r'postgresql://(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?/(.+)',
        s,
    )
    if not m:
        raise ValueError("Invalid connection string format")
    user, password, host, port, dbname = m.groups()
    params = {
        "host": host or "localhost",
        "port": int(port) if port else 5432,
        "dbname": dbname,
        "user": user or "postgres",
        "password": password or "",
    }
    # Parse query string for sslmode and other parameters
    if '?' in dbname:
        dbname_clean, query_string = dbname.split('?', 1)
        params["dbname"] = dbname_clean
        for param in query_string.split('&'):
            if '=' in param:
                key, value = param.split('=', 1)
                if key == 'sslmode':
                    params["sslmode"] = value
    return params


def _redact_password(s: str) -> str:
    """Redact password from a connection string for safe logging."""
    return re.sub(r':([^@]+)@', ':*****@', s)


def get_connection(connection_string: str | None = None) -> Any:
    """Get a PostgreSQL connection from connection string or environment variables.

    Connection string takes priority. If not provided, uses these env vars:
      PGHOST, PGPORT (default 5432), PGDATABASE, PGUSER, PGPASSWORD, PGSSLMODE (optional)

    Raises:
        ValueError: If required environment variables are missing.
    """
    if connection_string:
        try:
            params = parse_connection_string(connection_string)
        except ValueError as e:
            raise ValueError("Invalid connection string format") from e
    else:
        missing = []
        for name in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD"):
            if not os.environ.get(name):
                missing.append(name)
        if missing:
            names = ", ".join(missing)
            raise ValueError(
                f"Missing required environment variable(s): {names}\n"
                "  Set them in your shell or use --connection\n"
                "  Example: export PGHOST=localhost PGUSER=myuser PGPASSWORD=..."
            )
        params = {
            "host": os.environ["PGHOST"],
            "port": int(os.environ.get("PGPORT", "5432")),
            "dbname": os.environ["PGDATABASE"],
            "user": os.environ["PGUSER"],
            "password": os.environ["PGPASSWORD"],
        }
        # Add sslmode from environment if set
        sslmode = os.environ.get("PGSSLMODE")
        if sslmode:
            params["sslmode"] = sslmode
    # Set 30-second statement timeout to prevent runaway queries
    params.setdefault("options", "-c statement_timeout=30000")
    return psycopg2.connect(**params)


class ConnectionPool:
    """Thread-safe connection pool wrapper around psycopg2.

    Provides a persistent pool of connections for long-running processes
    like the MCP server. Connections are automatically returned to the
    pool when closed.

    Usage:
        pool = ConnectionPool(minconn=1, maxconn=5)
        conn = pool.getconn()
        # use conn...
        conn.close()  # returns to pool
        pool.closeall()  # at shutdown
    """

    def __init__(
        self,
        minconn: int = 1,
        maxconn: int = 5,
        connection_string: str | None = None,
        host: str | None = None,
        port: int | None = None,
        dbname: str | None = None,
        user: str | None = None,
        password: str | None = None,
        options: str | None = None,
    ):
        """Create a connection pool.

        Args:
            minconn: Minimum number of connections to keep in the pool.
            maxconn: Maximum number of connections in the pool.
            connection_string: PostgreSQL connection string. If None, uses env vars.
            options: PostgreSQL connection options (e.g. "-c statement_timeout=30000").
        """
        self.connection_string = connection_string
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
        if self.host and self.dbname and self.user:
            return psycopg2.pool.ThreadedConnectionPool(
                self._minconn, self._maxconn,
                host=self.host,
                port=self.port or int(os.environ.get("PGPORT", "5432")),
                dbname=self.dbname,
                user=self.user,
                password=self.password or "",
                options=self.options,
            )
        if self.connection_string:
            try:
                params = parse_connection_string(self.connection_string)
            except ValueError as e:
                raise ValueError("Invalid connection string format") from e
            params.setdefault("options", self.options)
            return psycopg2.pool.ThreadedConnectionPool(
                self._minconn, self._maxconn, **params
            )
        return psycopg2.pool.ThreadedConnectionPool(
            self._minconn, self._maxconn,
            host=os.environ.get("PGHOST"),
            port=int(os.environ.get("PGPORT", "5432")),
            dbname=os.environ.get("PGDATABASE"),
            user=os.environ.get("PGUSER"),
            password=os.environ.get("PGPASSWORD"),
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
    "parse_connection_string",
    "get_connection",
    "ConnectionPool",
]
