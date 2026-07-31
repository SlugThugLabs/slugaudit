"""Schema service — database schema initialization."""

import os
import re
import sysconfig
from typing import Any

_DOLLAR_TAG_RE = re.compile(r"\$([A-Za-z_][A-Za-z0-9_]*)?\$")


def _split_sql_statements(sql: str) -> list[str]:
    """Split a .sql file into individually-executable statements.

    A bare ``sql.split(";")`` breaks the moment schema.sql contains a
    semicolon that isn't a statement terminator: inside a string literal,
    a quoted identifier, a ``--``/``/* */`` comment, or (critically for any
    future trigger/function migration) a ``$$``- or ``$tag$``-delimited
    dollar-quoted body. This walks the text tracking which of those regions
    it's in and only splits on a ``;`` outside all of them. Comment and
    string content is preserved verbatim in the returned statements — this
    only changes *where* the cuts happen, not what text ends up in each
    statement.
    """
    statements: list[str] = []
    current: list[str] = []
    i = 0
    n = len(sql)
    in_single = False
    in_double = False
    block_comment_depth = 0
    dollar_tag: str | None = None

    while i < n:
        ch = sql[i]

        if dollar_tag is not None:
            if sql.startswith(dollar_tag, i):
                current.append(dollar_tag)
                i += len(dollar_tag)
                dollar_tag = None
            else:
                current.append(ch)
                i += 1
            continue

        if block_comment_depth > 0:
            if sql.startswith("/*", i):
                block_comment_depth += 1
                current.append("/*")
                i += 2
            elif sql.startswith("*/", i):
                block_comment_depth -= 1
                current.append("*/")
                i += 2
            else:
                current.append(ch)
                i += 1
            continue

        if in_single:
            current.append(ch)
            i += 1
            if ch == "'":
                if sql[i:i + 1] == "'":
                    current.append("'")
                    i += 1
                else:
                    in_single = False
            continue

        if in_double:
            current.append(ch)
            i += 1
            if ch == '"':
                if sql[i:i + 1] == '"':
                    current.append('"')
                    i += 1
                else:
                    in_double = False
            continue

        if sql.startswith("--", i):
            end = sql.find("\n", i)
            end = n if end == -1 else end + 1
            current.append(sql[i:end])
            i = end
            continue

        if sql.startswith("/*", i):
            block_comment_depth = 1
            current.append("/*")
            i += 2
            continue

        if ch == "'":
            in_single = True
            current.append(ch)
            i += 1
            continue

        if ch == '"':
            in_double = True
            current.append(ch)
            i += 1
            continue

        if ch == "$":
            match = _DOLLAR_TAG_RE.match(sql, i)
            if match:
                dollar_tag = match.group(0)
                current.append(dollar_tag)
                i += len(dollar_tag)
                continue

        if ch == ";":
            statement = "".join(current).strip()
            if statement:
                statements.append(statement)
            current = []
            i += 1
            continue

        current.append(ch)
        i += 1

    tail = "".join(current).strip()
    if tail:
        statements.append(tail)
    return statements


class SchemaService:
    """Handles database schema initialization (idempotent).

    Used by both CLI and MCP server to ensure the schema exists.
    """

    def __init__(self, schema_path: str | None = None):
        """Create a schema service.

        Args:
            schema_path: Path to schema.sql. If None, looks in the
                         same directory as this module.
        """
        if schema_path:
            self.schema_path = schema_path
        else:
            development_path = os.path.join(
                os.path.dirname(os.path.abspath(__file__)),
                "..",
                "schema.sql",
            )
            installed_path = os.path.join(
                sysconfig.get_path("data"),
                "share",
                "slugaudit-mcp",
                "schema.sql",
            )
            self.schema_path = (
                development_path if os.path.exists(development_path) else installed_path
            )

    SCHEMA_VERSION = 4
    _ADVISORY_LOCK_KEY = 8_627_041_113

    def initialize(self, conn: Any, logger: Any = None) -> bool:
        """Initialize the database schema (idempotent).

        Args:
            conn: Database connection.
            logger: Optional logger for warnings.

        Returns:
            True when the expected schema version is installed.
        """
        if not os.path.exists(self.schema_path):
            raise FileNotFoundError(f"schema.sql not found at {self.schema_path}")

        with open(self.schema_path) as f:
            schema_sql = f.read()

        statements = _split_sql_statements(schema_sql)
        cur = conn.cursor()
        try:
            # Serialize startup migrations across MCP workers/processes. This is
            # a transaction-scoped lock and is released by commit or rollback.
            cur.execute("SELECT pg_advisory_xact_lock(%s)", (self._ADVISORY_LOCK_KEY,))

            for index, stmt in enumerate(statements):
                savepoint = f"slugaudit_schema_{index}"
                cur.execute(f"SAVEPOINT {savepoint}")
                try:
                    cur.execute(stmt)
                except Exception as exc:
                    # The historical schema uses ALTER TABLE ADD CONSTRAINT,
                    # which has no IF NOT EXISTS form. Roll back only that
                    # statement when the constraint is already installed.
                    cur.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
                    if not self._is_duplicate_object(exc):
                        raise
                finally:
                    cur.execute(f"RELEASE SAVEPOINT {savepoint}")

            cur.execute(
                "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE version = %s)",
                (self.SCHEMA_VERSION,),
            )
            row = cur.fetchone()
            if not row or not row[0]:
                raise RuntimeError(
                    f"Schema migration {self.SCHEMA_VERSION} was not recorded"
                )
            conn.commit()
        except BaseException:
            conn.rollback()
            raise
        finally:
            cur.close()
        if logger:
            logger.info(
                "Database schema initialized at version %s", self.SCHEMA_VERSION
            )
        return True

    @staticmethod
    def _is_duplicate_object(exc: Exception) -> bool:
        """Return True only for PostgreSQL duplicate-object failures."""
        pgcode = getattr(exc, "pgcode", None)
        if pgcode in {"42710", "42P07"}:  # duplicate_object, duplicate_table
            return True
        message = str(exc).lower()
        return "already exists" in message or "duplicate object" in message

    def is_current(self, conn: Any) -> bool:
        """Check that the database has the exact schema version we require."""
        cur = conn.cursor()
        try:
            cur.execute(
                "SELECT EXISTS ("
                "SELECT 1 FROM information_schema.tables "
                "WHERE table_schema = 'public' AND table_name = 'schema_migrations'"
                ")"
            )
            row = cur.fetchone()
            if not row or not row[0]:
                return False
            cur.execute(
                "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE version = %s)",
                (self.SCHEMA_VERSION,),
            )
            row = cur.fetchone()
            return bool(row and row[0])
        except Exception:
            conn.rollback()
            return False
        finally:
            cur.close()


__all__ = ["SchemaService"]
