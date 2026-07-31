"""Schema service — SQLite variant for the zero-config local backend.

Simpler than services/schema_service.py's PostgreSQL version by construction:
sqlite_schema.sql is a brand-new schema with no legacy ALTER TABLE migrations
to recover from, so every statement in it is already idempotent
(``CREATE TABLE IF NOT EXISTS``, ``CREATE INDEX IF NOT EXISTS``,
``INSERT OR IGNORE``) and sqlite3's ``executescript`` runs a full
multi-statement file natively — no dollar-quote/string-aware statement
splitter needed the way the PostgreSQL side requires one.
"""

import os
import sqlite3
from typing import Any


class SqliteSchemaService:
    """Handles SQLite schema initialization (idempotent)."""

    SCHEMA_VERSION = 1

    def __init__(self, schema_path: str | None = None):
        if schema_path:
            self.schema_path = schema_path
        else:
            self.schema_path = os.path.join(
                os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                "sqlite_schema.sql",
            )

    def initialize(self, conn: sqlite3.Connection, logger: Any = None) -> bool:
        """Initialize the SQLite schema (idempotent).

        Returns:
            True when the expected schema version is installed.
        """
        if not os.path.exists(self.schema_path):
            raise FileNotFoundError(f"sqlite_schema.sql not found at {self.schema_path}")

        with open(self.schema_path) as f:
            schema_sql = f.read()

        conn.executescript(schema_sql)

        if not self.is_current(conn):
            raise RuntimeError(
                f"SQLite schema migration {self.SCHEMA_VERSION} was not recorded"
            )
        if logger:
            logger.info(
                "SQLite schema initialized at version %s", self.SCHEMA_VERSION
            )
        return True

    def is_current(self, conn: sqlite3.Connection) -> bool:
        """Check that the database has the exact schema version we require."""
        try:
            cur = conn.execute(
                "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE version = ?)",
                (self.SCHEMA_VERSION,),
            )
            row = cur.fetchone()
            return bool(row and row[0])
        except sqlite3.Error:
            return False


__all__ = ["SqliteSchemaService"]
