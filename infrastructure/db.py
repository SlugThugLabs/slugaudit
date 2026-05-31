"""PostgreSQL connection and pooling infrastructure.

Provides connection management with support for connection strings,
environment variables, connection pooling, and credential-safe
error handling.
"""

import os
import re
import threading
from typing import Optional

import psycopg2


def parse_connection_string(s: str) -> dict:
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


def get_connection(connection_string: Optional[str] = None):
    """Get a PostgreSQL connection from connection string or environment variables.

    Connection string takes priority. If not provided, uses these env vars:
      PGHOST, PGPORT (default 5432), PGDATABASE, PGUSER, PGPASSWORD, PGSSLMODE (optional)

    Raises:
        ValueError: If required environment variables are missing.
    """
    if connection_string:
        try:
            params = parse_connection_string(connection_string)
        except ValueError:
            raise ValueError("Invalid connection string format")
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
    return psycopg2.connect(**params)


class ConnectionPool:
    """Simple connection pool wrapper around psycopg2.pool.SimpleConnectionPool.

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

    def __init__(self, minconn: int = 1, maxconn: int = 5, connection_string: Optional[str] = None):
        """Create a connection pool.

        Args:
            minconn: Minimum number of connections to keep in the pool.
            maxconn: Maximum number of connections in the pool.
            connection_string: PostgreSQL connection string. If None, uses env vars.
        """
        self.connection_string = connection_string
        self._pool = None
        self._minconn = minconn
        self._maxconn = maxconn
        self._lock = threading.Lock()

    @property
    def pool(self):
        if self._pool is None:
            with self._lock:
                if self._pool is None:
                    if self.connection_string:
                        try:
                            params = parse_connection_string(self.connection_string)
                        except ValueError:
                            raise ValueError("Invalid connection string format")
                        self._pool = psycopg2.pool.SimpleConnectionPool(
                            self._minconn, self._maxconn, **params
                        )
                    else:
                        # Use env vars
                        self._pool = psycopg2.pool.SimpleConnectionPool(
                            self._minconn, self._maxconn,
                            host=os.environ.get("PGHOST"),
                            port=int(os.environ.get("PGPORT", "5432")),
                            dbname=os.environ.get("PGDATABASE"),
                            user=os.environ.get("PGUSER"),
                            password=os.environ.get("PGPASSWORD"),
                        )
        return self._pool

    def getconn(self):
        """Get a connection from the pool."""
        return self.pool.getconn()

    def putconn(self, conn):
        """Return a connection to the pool."""
        self.pool.putconn(conn)

    def closeall(self):
        """Close all connections and destroy the pool."""
        if self._pool is not None:
            self._pool.closeall()
            self._pool = None


__all__ = [
    "parse_connection_string",
    "get_connection",
    "ConnectionPool",
]
