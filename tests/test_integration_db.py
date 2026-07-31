"""Live-PostgreSQL integration tests.

Every other test module in this suite mocks the database connection and
cursor — real behavior of schema.sql (the dollar-quote/string-aware statement
splitter added to services/schema_service.py, the advisory lock, savepoint
recovery) and of infrastructure/db.py's ThreadedConnectionPool is otherwise
never exercised against an actual server.

Skipped unless SLUGAUDIT_RUN_DB_TESTS=1 is set, so a normal `pytest -q` run
never touches a network database. When enabled, it never targets the real
audit data: it creates and drops its own PostgreSQL schema
(SLUGAUDIT_TEST_SCHEMA, default "slugaudit_test") via search_path, isolated
from whatever "public" (or any other schema) holds. Point
PGHOST/PGPORT/PGDATABASE/PGUSER/PGPASSWORD at a database you're allowed to
create and drop a throwaway schema in — never a database holding real audit
evidence, and never rely on this to also validate against "public".
"""

import os
import unittest
import uuid
from typing import Any

RUN_DB_TESTS = os.environ.get("SLUGAUDIT_RUN_DB_TESTS") == "1"
TEST_SCHEMA = os.environ.get("SLUGAUDIT_TEST_SCHEMA", "slugaudit_test")

_SKIP_REASON = (
    "Set SLUGAUDIT_RUN_DB_TESTS=1 (plus PGHOST/PGPORT/PGDATABASE/PGUSER/"
    "PGPASSWORD pointing at a database you can create/drop a schema in) to "
    "run live-Postgres integration tests."
)


def _connection_kwargs() -> dict[str, Any]:
    return {
        "host": os.environ.get("PGHOST", "localhost"),
        "port": int(os.environ.get("PGPORT", "5432")),
        "dbname": os.environ.get("PGDATABASE", ""),
        "user": os.environ.get("PGUSER", ""),
        "password": os.environ.get("PGPASSWORD", ""),
    }


@unittest.skipUnless(RUN_DB_TESTS, _SKIP_REASON)
class TestSchemaServiceAgainstRealPostgres(unittest.TestCase):
    """Exercises the actual shipped schema.sql against real PostgreSQL."""

    @classmethod
    def setUpClass(cls) -> None:
        import psycopg2

        cls.conn = psycopg2.connect(
            **_connection_kwargs(), options=f"-c search_path={TEST_SCHEMA}"
        )
        with cls.conn.cursor() as cur:
            cur.execute(f"DROP SCHEMA IF EXISTS {TEST_SCHEMA} CASCADE")
            cur.execute(f"CREATE SCHEMA {TEST_SCHEMA}")
        cls.conn.commit()

    @classmethod
    def tearDownClass(cls) -> None:
        with cls.conn.cursor() as cur:
            cur.execute(f"DROP SCHEMA IF EXISTS {TEST_SCHEMA} CASCADE")
        cls.conn.commit()
        cls.conn.close()

    def test_schema_initializes_and_is_idempotent(self) -> None:
        from services.schema_service import SchemaService

        service = SchemaService()
        self.assertTrue(service.initialize(self.conn))
        # Re-running must not fail: idempotent CREATE ... IF NOT EXISTS plus
        # savepoint recovery from the historical ALTER TABLE ADD CONSTRAINT
        # statements that have no IF NOT EXISTS form.
        self.assertTrue(service.initialize(self.conn))
        self.assertTrue(service.is_current(self.conn))

    def test_advisory_lock_key_is_actually_acquirable(self) -> None:
        from services.schema_service import SchemaService

        service = SchemaService()
        service.initialize(self.conn)
        with self.conn.cursor() as cur:
            cur.execute(
                "SELECT pg_try_advisory_xact_lock(%s)",
                (service._ADVISORY_LOCK_KEY,),
            )
            self.assertTrue(cur.fetchone()[0])
        self.conn.rollback()

    def test_project_and_file_repository_round_trip_against_real_tables(
        self,
    ) -> None:
        from services.schema_service import SchemaService
        from repositories import FileRepository, ProjectRepository, repository_transaction

        SchemaService().initialize(self.conn)

        project_repo = ProjectRepository(self.conn, auto_commit=False)
        file_repo = FileRepository(self.conn, auto_commit=False)
        repo_path = f"/tmp/slugaudit-it-{uuid.uuid4()}"

        with repository_transaction(self.conn):
            project_id = project_repo.get_or_create(
                name="integration-test", language="python", repo_path=repo_path
            )
            file_id, created = file_repo.upsert(
                project_id=project_id,
                relpath="a.py",
                file_hash="0" * 64,
                file_size=10,
                mtime="2026-01-01T00:00:00+00:00",
                signatures=[],
                content="print(1)\n",
                force=True,
            )
            self.assertTrue(created)
            self.assertIsNotNone(file_id)

        stats = file_repo.get_file_stats(project_id)
        self.assertEqual(stats["total_files"], 1)
        # files.size is BIGINT; PostgreSQL's SUM(bigint) returns numeric,
        # which psycopg2 decodes as Decimal unless get_file_stats casts it.
        # This broke audit_brief's json.dumps in production — assert the
        # real type here, against a real server, not just a mock.
        self.assertIs(type(stats["total_bytes"]), int)
        import json
        json.dumps(stats)  # must not raise

        status = project_repo.get_status(project_id)
        self.assertIs(type(status["total_size"]), int)
        json.dumps(status)  # must not raise

        rows = file_repo.get_file_contents(project_id, ["a.py"])
        self.assertEqual(rows, [("a.py", "print(1)\n")])


@unittest.skipUnless(RUN_DB_TESTS, _SKIP_REASON)
class TestConnectionPoolAgainstRealPostgres(unittest.TestCase):
    """The pool wrapper against a real ThreadedConnectionPool, not a MagicMock."""

    def test_get_and_put_connection_round_trips(self) -> None:
        from infrastructure.db import ConnectionPool

        pool = ConnectionPool(minconn=1, maxconn=2, **_connection_kwargs())
        try:
            conn = pool.getconn()
            try:
                with conn.cursor() as cur:
                    cur.execute("SELECT 1")
                    self.assertEqual(cur.fetchone()[0], 1)
            finally:
                pool.putconn(conn)
        finally:
            pool.closeall()


if __name__ == "__main__":
    unittest.main()
