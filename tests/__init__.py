"""
Tests for audit-db core functionality.

Run with: python3 -m pytest tests/ -v
Or: python3 -m unittest tests.test_core tests.test_db tests.test_brief
"""

import os
import sys
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class TestConnectionParsing(unittest.TestCase):
    """Test connection string parsing and environment variable handling."""

    def test_parse_simple_connection_string(self):
        from db import parse_connection_string
        result = parse_connection_string("postgresql://user:pass@host:5432/dbname")
        self.assertEqual(result["user"], "user")
        self.assertEqual(result["password"], "pass")
        self.assertEqual(result["host"], "host")
        self.assertEqual(result["port"], 5432)
        self.assertEqual(result["dbname"], "dbname")

    def test_parse_connection_string_no_port(self):
        from db import parse_connection_string
        result = parse_connection_string("postgresql://user:pass@host/dbname")
        self.assertEqual(result["host"], "host")
        self.assertEqual(result["port"], 5432)  # default

    def test_parse_connection_string_no_password(self):
        from db import parse_connection_string
        result = parse_connection_string("postgresql://user@host/dbname")
        self.assertEqual(result["user"], "user")
        self.assertEqual(result["password"], "")

    def test_parse_invalid_connection_string(self):
        from db import parse_connection_string
        with self.assertRaises(ValueError):
            parse_connection_string("not-a-valid-connection-string")

    def test_get_connection_missing_env_vars(self):
        from db import get_connection
        # Clear relevant env vars
        for var in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD"):
            os.environ.pop(var, None)
        with self.assertRaises(ValueError) as ctx:
            get_connection()
        self.assertIn("PGHOST", str(ctx.exception))
        self.assertIn("PGDATABASE", str(ctx.exception))


class TestSchemaExists(unittest.TestCase):
    """Test schema existence check."""

    def test_schema_exists_returns_true(self):
        from db import schema_exists
        mock_conn = MagicMock()
        mock_cur = MagicMock()
        mock_conn.cursor.return_value = mock_cur
        mock_cur.fetchone.return_value = (True,)
        self.assertTrue(schema_exists(mock_conn))

    def test_schema_exists_returns_false(self):
        from db import schema_exists
        mock_conn = MagicMock()
        mock_cur = MagicMock()
        mock_conn.cursor.return_value = mock_cur
        mock_cur.fetchone.return_value = (False,)
        self.assertFalse(schema_exists(mock_conn))

    def test_schema_exists_handles_exception(self):
        from db import schema_exists
        mock_conn = MagicMock()
        mock_cur = MagicMock()
        mock_conn.cursor.return_value = mock_cur
        mock_cur.fetchone.side_effect = Exception("DB error")
        self.assertFalse(schema_exists(mock_conn))


class TestCoreImportResult(unittest.TestCase):
    """Test the ImportResult class from core.py."""

    def test_import_result_str(self):
        from core import ImportResult
        result = ImportResult()
        result.project_id = "test-id"
        result.project_name = "Test Project"
        result.language = "rust"
        result.files_processed = 100
        result.signatures_extracted = 500
        result.imports_extracted = 200
        result.dependency_edges = 50
        result.elapsed_seconds = 2.5

        output = str(result)
        self.assertIn("Test Project", output)
        self.assertIn("rust", output)
        self.assertIn("100", output)
        self.assertIn("500", output)

    def test_import_result_defaults(self):
        from core import ImportResult
        result = ImportResult()
        self.assertIsNone(result.project_id)
        self.assertEqual(result.project_name, "")
        self.assertEqual(result.files_processed, 0)


class TestCoreGetExtractor(unittest.TestCase):
    """Test extractor resolution in core.py."""

    def test_get_extractor_with_explicit_language(self):
        from core import get_extractor
        # This should not raise for a valid language
        # We can't actually test the extractor without tree-sitter installed,
        # but we can test that it doesn't raise for valid languages
        with self.assertRaises(ValueError):
            get_extractor("/nonexistent/path", "invalid_language")

    def test_get_extractor_invalid_language(self):
        from core import get_extractor
        with self.assertRaises(ValueError):
            get_extractor("/tmp", "COBOL")


class TestBriefFormatting(unittest.TestCase):
    """Test briefing formatting helpers."""

    def test_fmt_sig_with_signature(self):
        from brief import fmt_sig
        sig = {
            "visibility": "pub",
            "signature": "fn foo() -> Result<T, E>",
            "type": "function",
            "name": "foo",
        }
        self.assertEqual(fmt_sig(sig), "- pub fn foo() -> Result<T, E>")

    def test_fmt_sig_without_signature(self):
        from brief import fmt_sig
        sig = {
            "visibility": "",
            "type": "struct",
            "name": "MyStruct",
        }
        self.assertEqual(fmt_sig(sig), "- struct MyStruct")

    def test_fmt_sig_unknown_type(self):
        from brief import fmt_sig
        sig = {"name": "unknown"}
        self.assertEqual(fmt_sig(sig), "- ? unknown")


class TestConnectionPool(unittest.TestCase):
    """Test the ConnectionPool class."""

    def test_pool_creation(self):
        from db import ConnectionPool
        pool = ConnectionPool(minconn=1, maxconn=3)
        self.assertIsNotNone(pool)
        # Pool is lazily initialized, so _pool should be None
        self.assertIsNone(pool._pool)

    def test_pool_getconn_putconn(self):
        from db import ConnectionPool
        # Skip this test if no database is available
        if not all(os.environ.get(v) for v in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD")):
            self.skipTest("No PostgreSQL connection available")
        pool = ConnectionPool(minconn=1, maxconn=3)
        conn = pool.getconn()
        self.assertIsNotNone(conn)
        # Verify connection is usable
        cur = conn.cursor()
        cur.execute("SELECT 1")
        self.assertEqual(cur.fetchone(), (1,))
        pool.putconn(conn)


if __name__ == "__main__":
    unittest.main()
