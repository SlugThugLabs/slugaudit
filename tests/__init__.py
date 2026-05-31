"""
Tests for slugaudit-mcp core functionality.

Run with: python3 -m pytest tests/ -v
Or: python3 -m unittest tests.test_core tests.test_db tests.test_brief
"""

import os
import sys
import json
import unittest
import threading
from datetime import datetime, timezone
from unittest.mock import MagicMock, patch, PropertyMock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


# =============================================================================
# Connection String Parsing Tests
# =============================================================================

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

    def test_parse_connection_string_with_sslmode(self):
        from db import parse_connection_string
        result = parse_connection_string("postgresql://user:pass@host:5432/dbname?sslmode=require")
        self.assertEqual(result["sslmode"], "require")
        self.assertEqual(result["dbname"], "dbname")

    def test_parse_connection_string_no_user_no_password(self):
        from db import parse_connection_string
        result = parse_connection_string("postgresql://host:5432/dbname")
        self.assertEqual(result["user"], "postgres")
        self.assertEqual(result["password"], "")

    def test_get_connection_with_connection_string(self):
        from db import get_connection
        with patch("db.psycopg2.connect") as mock_connect:
            mock_connect.return_value = MagicMock()
            get_connection("postgresql://u:p@h:5432/d")
            mock_connect.assert_called_once_with(
                host="h", port=5432, dbname="d", user="u", password="p"
            )

    def test_get_connection_with_env_vars(self):
        from db import get_connection
        with patch.dict(os.environ, {
            "PGHOST": "envhost",
            "PGDATABASE": "envdb",
            "PGUSER": "envuser",
            "PGPASSWORD": "envpass",
            "PGPORT": "5433",
        }):
            with patch("db.psycopg2.connect") as mock_connect:
                mock_connect.return_value = MagicMock()
                get_connection()
                mock_connect.assert_called_once_with(
                    host="envhost", port=5433, dbname="envdb",
                    user="envuser", password="envpass",
                )

    def test_get_connection_with_sslmode_env(self):
        from db import get_connection
        with patch.dict(os.environ, {
            "PGHOST": "h", "PGDATABASE": "d", "PGUSER": "u",
            "PGPASSWORD": "p", "PGSSLMODE": "require",
        }):
            with patch("db.psycopg2.connect") as mock_connect:
                mock_connect.return_value = MagicMock()
                get_connection()
                call_kwargs = mock_connect.call_args.kwargs
                self.assertEqual(call_kwargs.get("sslmode"), "require")


# =============================================================================
# Schema Exists Tests
# =============================================================================

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


# =============================================================================
# ImportResult Tests
# =============================================================================

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

    def test_import_result_str_all_fields(self):
        from core import ImportResult
        result = ImportResult()
        result.project_id = "abc-123"
        result.project_name = "MyApp"
        result.language = "python"
        result.files_processed = 42
        result.signatures_extracted = 200
        result.imports_extracted = 80
        result.dependency_edges = 35
        result.elapsed_seconds = 1.7
        output = str(result)
        self.assertIn("abc-123", output)
        self.assertIn("MyApp", output)
        self.assertIn("python", output)
        self.assertIn("42", output)
        self.assertIn("200", output)
        self.assertIn("80", output)
        self.assertIn("35", output)
        self.assertIn("1.7", output)

    def test_import_result_str_elapsed_formatting(self):
        from core import ImportResult
        result = ImportResult()
        result.elapsed_seconds = 0.05
        output = str(result)
        self.assertIn("0.1", output)  # formatted as 0.1 via :.1f

    def test_import_result_zero_values(self):
        from core import ImportResult
        result = ImportResult()
        result.project_name = "Empty"
        output = str(result)
        self.assertIn("Empty", output)
        self.assertIn("0", output)


# =============================================================================
# Extractor Tests
# =============================================================================

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

    def test_get_extractor_valid_languages(self):
        from core import get_extractor
        valid = ["rust", "python", "typescript", "go", "java", "c", "cpp", "ruby"]
        for lang in valid:
            try:
                extractor = get_extractor("/tmp", lang)
                # If it returns, the language is supported - that's good
            except ValueError as e:
                if "Unsupported language" in str(e):
                    self.fail(f"Language {lang} should be supported but got: {e}")
            except Exception:
                # Other exceptions (like tree-sitter not installed) are fine
                pass


# =============================================================================
# Brief Formatting Tests (fmt_sig)
# =============================================================================

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

    def test_fmt_sig_empty_dict(self):
        from brief import fmt_sig
        self.assertEqual(fmt_sig({}), "- ? ")

    def test_fmt_sig_no_visibility_with_signature(self):
        from brief import fmt_sig
        sig = {"signature": "fn bar(x: i32)", "type": "function", "name": "bar"}
        self.assertEqual(fmt_sig(sig), "- fn bar(x: i32)")

    def test_fmt_sig_private_visibility(self):
        from brief import fmt_sig
        sig = {"visibility": "priv", "signature": "fn hidden()", "type": "function", "name": "hidden"}
        self.assertEqual(fmt_sig(sig), "- priv fn hidden()")

    def test_fmt_sig_with_generic_params(self):
        from brief import fmt_sig
        sig = {
            "visibility": "pub",
            "signature": "fn map<T, U>(f: T) -> U",
            "type": "function",
            "name": "map",
            "generic_params": ["T", "U"],
        }
        self.assertEqual(fmt_sig(sig), "- pub fn map<T, U>(f: T) -> U")

    def test_fmt_sig_type_only(self):
        from brief import fmt_sig
        sig = {"type": "enum", "name": "Color"}
        self.assertEqual(fmt_sig(sig), "- enum Color")

    def test_fmt_sig_name_only(self):
        from brief import fmt_sig
        sig = {"name": "something"}
        self.assertEqual(fmt_sig(sig), "- ? something")

    def test_fmt_sig_empty_name_empty_type(self):
        from brief import fmt_sig
        sig = {"name": "", "type": ""}
        self.assertEqual(fmt_sig(sig), "-  ")


# =============================================================================
# ConnectionPool Tests
# =============================================================================

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

    def test_pool_creation_with_connection_string(self):
        from db import ConnectionPool
        pool = ConnectionPool(
            minconn=1, maxconn=2,
            connection_string="postgresql://u:p@h:5432/d"
        )
        self.assertIsNotNone(pool)
        self.assertIsNone(pool._pool)
        self.assertEqual(pool.connection_string, "postgresql://u:p@h:5432/d")

    def test_pool_closeall_before_init(self):
        from db import ConnectionPool
        pool = ConnectionPool(minconn=1, maxconn=3)
        # Should not raise when pool was never initialized
        pool.closeall()
        self.assertIsNone(pool._pool)

    def test_pool_lazy_initialization(self):
        from db import ConnectionPool
        pool = ConnectionPool(minconn=1, maxconn=3)
        self.assertIsNone(pool._pool)
        # Accessing pool property triggers initialization
        # Skip if no DB available
        if not all(os.environ.get(v) for v in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD")):
            self.skipTest("No PostgreSQL connection available")
        _ = pool.pool
        self.assertIsNotNone(pool._pool)

    def test_pool_thread_safety_double_init(self):
        """Test that two threads don't create two pools simultaneously."""
        from db import ConnectionPool
        if not all(os.environ.get(v) for v in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD")):
            self.skipTest("No PostgreSQL connection available")
        pool = ConnectionPool(minconn=1, maxconn=3)
        errors = []

        def access_pool():
            try:
                _ = pool.pool
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=access_pool) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(errors, [])
        self.assertIsNotNone(pool._pool)

    def test_pool_getconn_putconn_mocked(self):
        from db import ConnectionPool
        import psycopg2.pool
        mock_pool = MagicMock()
        mock_conn = MagicMock()
        mock_pool.getconn.return_value = mock_conn
        with patch.object(psycopg2.pool, "SimpleConnectionPool", return_value=mock_pool):
            pool = ConnectionPool(minconn=1, maxconn=3, connection_string="postgresql://u:p@h:5432/d")
            conn = pool.getconn()
            self.assertIs(conn, mock_conn)
            mock_pool.getconn.assert_called_once()
            pool.putconn(conn)
            mock_pool.putconn.assert_called_once_with(mock_conn)

    def test_pool_with_env_vars_mocked(self):
        from db import ConnectionPool
        import psycopg2.pool
        mock_pool = MagicMock()
        mock_conn = MagicMock()
        mock_pool.getconn.return_value = mock_conn
        with patch.dict(os.environ, {
            "PGHOST": "eh", "PGDATABASE": "ed", "PGUSER": "eu", "PGPASSWORD": "ep"
        }):
            with patch.object(psycopg2.pool, "SimpleConnectionPool", return_value=mock_pool) as m:
                pool = ConnectionPool(minconn=2, maxconn=5)
                conn = pool.getconn()
                self.assertIs(conn, mock_conn)
                m.assert_called_once()
                call_kwargs = m.call_args.kwargs
                self.assertEqual(call_kwargs["host"], "eh")
                self.assertEqual(call_kwargs["dbname"], "ed")


# =============================================================================
# upsert_file Tests
# =============================================================================

class TestUpsertFile(unittest.TestCase):
    """Test the upsert_file function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_upsert_file_new_file(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        # First fetchone for SELECT (None = not found), second for INSERT RETURNING
        cur.fetchone.side_effect = [None, ("new-fid",)]

        with patch("db.json.dumps", return_value='[]') as mock_dumps:
            fid, was_updated = upsert_file(
                conn, "proj-1", "src/main.rs", "hash123", 1024,
                datetime.now(timezone.utc), [{"type": "fn", "name": "main"}]
            )
            mock_dumps.assert_called_once()

        self.assertEqual(fid, "new-fid")
        # Should have done INSERT
        insert_calls = [c for c in cur.execute.call_args_list
                        if "INSERT INTO files" in str(c)]
        self.assertEqual(len(insert_calls), 1)
        self.assertTrue(was_updated)

    def test_upsert_file_existing_unchanged(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.return_value = ("file-123", "hash123")  # Same hash

        fid, was_updated = upsert_file(
            conn, "proj-1", "src/main.rs", "hash123", 1024,
            datetime.now(timezone.utc), []
        )
        self.assertEqual(fid, "file-123")
        self.assertFalse(was_updated)
        # Should NOT have done UPDATE or INSERT
        all_calls = " ".join(str(c) for c in cur.execute.call_args_list)
        self.assertNotIn("UPDATE", all_calls)
        self.assertNotIn("INSERT", all_calls)

    def test_upsert_file_existing_changed(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.return_value = ("file-456", "old_hash")  # Existing with different hash

        fid, was_updated = upsert_file(
            conn, "proj-1", "src/main.rs", "new_hash", 2048,
            datetime.now(timezone.utc), [{"type": "fn", "name": "helper"}]
        )
        self.assertEqual(fid, "file-456")
        self.assertTrue(was_updated)
        # Should have done UPDATE
        update_calls = [c for c in cur.execute.call_args_list
                        if "UPDATE files" in str(c)]
        self.assertEqual(len(update_calls), 1)

    def test_upsert_file_force_updates_even_if_unchanged(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.return_value = ("file-789", "hash123")

        fid, was_updated = upsert_file(
            conn, "proj-1", "src/main.rs", "hash123", 1024,
            datetime.now(timezone.utc), [], force=True
        )
        self.assertTrue(was_updated)
        # Force should trigger UPDATE even though hash matches
        update_calls = [c for c in cur.execute.call_args_list
                        if "UPDATE files" in str(c)]
        self.assertEqual(len(update_calls), 1)

    def test_upsert_file_no_signatures(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [None, ("new-fid",)]

        # When signatures list is empty, json.dumps is NOT called
        fid, was_updated = upsert_file(
            conn, "proj-1", "empty.rs", "hash0", 0,
            datetime.now(timezone.utc), []
        )
        self.assertEqual(fid, "new-fid")
        self.assertTrue(was_updated)

    def test_upsert_file_commits(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [None, ("new-fid",)]

        upsert_file(conn, "proj-1", "a.rs", "h1", 100, datetime.now(timezone.utc), [])
        conn.commit.assert_called_once()

    def test_upsert_file_closes_cursor(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [None, ("new-fid",)]

        upsert_file(conn, "proj-1", "a.rs", "h1", 100, datetime.now(timezone.utc), [])
        cur.close.assert_called_once()

    def test_upsert_file_insert_returns_id(self):
        from db import upsert_file
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            None,  # SELECT returns None
            ("new-file-id",),  # INSERT RETURNING id
        ]

        fid, was_updated = upsert_file(
            conn, "proj-1", "new.rs", "hash", 50, datetime.now(timezone.utc), []
        )
        self.assertEqual(fid, "new-file-id")
        self.assertTrue(was_updated)


# =============================================================================
# delete_removed_files Tests
# =============================================================================

class TestDeleteRemovedFiles(unittest.TestCase):
    """Test the delete_removed_files function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_delete_removed_files_no_files_to_delete(self):
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("f1", "src/a.rs")]

        delete_removed_files(conn, "proj-1", {"src/a.rs"})

        # Should not call DELETE
        delete_calls = [c for c in cur.execute.call_args_list
                        if "DELETE" in str(c)]
        self.assertEqual(len(delete_calls), 0)

    def test_delete_removed_files_deletes_one_file(self):
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("f1", "src/old.rs"), ("f2", "src/kept.rs")]

        delete_removed_files(conn, "proj-1", {"src/kept.rs"})

        # Should delete dependency_edges for f1 (both columns)
        de_delete = [c for c in cur.execute.call_args_list
                     if "DELETE FROM dependency_edges" in str(c)]
        self.assertEqual(len(de_delete), 1)

        # Should delete file_imports for f1
        fi_delete = [c for c in cur.execute.call_args_list
                     if "DELETE FROM file_imports" in str(c)]
        self.assertEqual(len(fi_delete), 1)

        # Should delete file_staleness for f1
        fs_delete = [c for c in cur.execute.call_args_list
                     if "DELETE FROM file_staleness" in str(c)]
        self.assertEqual(len(fs_delete), 1)

        # Should delete findings for f1
        fn_delete = [c for c in cur.execute.call_args_list
                     if "DELETE FROM findings" in str(c)]
        self.assertEqual(len(fn_delete), 1)

        # Should delete file itself
        file_delete = [c for c in cur.execute.call_args_list
                       if "DELETE FROM files" in str(c)]
        self.assertEqual(len(file_delete), 1)

    def test_delete_removed_files_multiple_deletions(self):
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [
            ("f1", "a.rs"), ("f2", "b.rs"), ("f3", "c.rs")
        ]

        delete_removed_files(conn, "proj-1", set())  # All should be deleted

        # 3 files * 5 delete statements each = 15
        delete_calls = [c for c in cur.execute.call_args_list
                        if "DELETE" in str(c)]
        self.assertEqual(len(delete_calls), 15)

    def test_delete_removed_files_empty_project(self):
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = []

        delete_removed_files(conn, "proj-empty", {"nonexistent.rs"})

        # No files found, no deletes
        delete_calls = [c for c in cur.execute.call_args_list
                        if "DELETE" in str(c)]
        self.assertEqual(len(delete_calls), 0)

    def test_delete_removed_files_cascading_order(self):
        """Verify deletes happen in correct order: dependent tables first, then file."""
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("f1", "gone.rs")]

        delete_removed_files(conn, "proj-1", set())

        # Get all delete calls in order
        calls = [str(c) for c in cur.execute.call_args_list]
        # Find indices of key deletes
        de_idx = next(i for i, c in enumerate(calls) if "DELETE FROM dependency_edges" in c)
        fi_idx = next(i for i, c in enumerate(calls) if "DELETE FROM file_imports" in c)
        fs_idx = next(i for i, c in enumerate(calls) if "DELETE FROM file_staleness" in c)
        fn_idx = next(i for i, c in enumerate(calls) if "DELETE FROM findings" in c)
        file_idx = next(i for i, c in enumerate(calls) if "DELETE FROM files" in c)

        # File delete should be last
        self.assertGreater(file_idx, de_idx)
        self.assertGreater(file_idx, fi_idx)
        self.assertGreater(file_idx, fs_idx)
        self.assertGreater(file_idx, fn_idx)

    def test_delete_removed_files_commits(self):
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("f1", "a.rs")]

        delete_removed_files(conn, "proj-1", set())

        conn.commit.assert_called_once()

    def test_delete_removed_files_closes_cursor(self):
        from db import delete_removed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = []

        delete_removed_files(conn, "proj-1", set())

        cur.close.assert_called_once()


# =============================================================================
# insert_imports Tests
# =============================================================================

class TestInsertImports(unittest.TestCase):
    """Test the insert_imports function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_insert_imports_basic(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        imports = [
            {"import_text": "use std::io;", "import_type": "internal"},
            {"import_text": "use crate::foo;", "import_type": "internal"},
        ]
        insert_imports(conn, "proj-1", "file-1", imports)

        self.assertEqual(cur.execute.call_count, 2)
        conn.commit.assert_called_once()

    def test_insert_imports_force_clears_old(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        imports = [{"import_text": "use new::thing;"}]
        insert_imports(conn, "proj-1", "file-1", imports, force=True)

        delete_call = [c for c in cur.execute.call_args_list
                       if "DELETE FROM file_imports" in str(c)]
        self.assertEqual(len(delete_call), 1)

    def test_insert_imports_no_force_keeps_old(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        imports = [{"import_text": "use new::thing;"}]
        insert_imports(conn, "proj-1", "file-1", imports, force=False)

        delete_calls = [c for c in cur.execute.call_args_list
                        if "DELETE" in str(c)]
        self.assertEqual(len(delete_calls), 0)

    def test_insert_imports_empty_list(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        insert_imports(conn, "proj-1", "file-1", [])

        cur.execute.assert_not_called()
        conn.commit.assert_called_once()

    def test_insert_imports_optional_fields(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        imports = [
            {
                "import_text": "from foo import bar",
                "resolved_path": "src/foo/bar.py",
                "import_type": "external",
                "line_start": 10,
                "line_end": 10,
            }
        ]
        insert_imports(conn, "proj-1", "file-1", imports)

        # Verify the INSERT was called with correct values
        call_args = cur.execute.call_args[0]
        params = call_args[1]  # The parameter tuple
        self.assertIn("src/foo/bar.py", params)
        self.assertIn("external", params)
        self.assertIn(10, params)

    def test_insert_imports_missing_optional_fields(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        imports = [{"import_text": "use something;"}]
        insert_imports(conn, "proj-1", "file-1", imports)

        # Should use defaults: resolved_path=None, import_type="internal", line_start=None, line_end=None
        call_args = cur.execute.call_args[0]
        params = call_args[1]  # The parameter tuple
        self.assertIn("internal", params)

    def test_insert_imports_closes_cursor(self):
        from db import insert_imports
        conn, cur = self._make_mock_conn()

        insert_imports(conn, "proj-1", "file-1", [])

        cur.close.assert_called_once()


# =============================================================================
# build_dependency_edges Tests
# =============================================================================

class TestBuildDependencyEdges(unittest.TestCase):
    """Test the build_dependency_edges function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_build_dependency_edges_basic(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()

        # Files in project
        cur.fetchall.side_effect = [
            [("f1", "src/a.rs"), ("f2", "src/b.rs")],  # file_map
            [("imp1", "f1", "use crate::b;", "src/a.rs")],  # imports to resolve
        ]
        cur.rowcount = 1  # Simulate successful insert

        mock_importer = MagicMock()
        mock_importer.resolve_import.return_value = "src/b.rs"

        edges = build_dependency_edges(conn, "proj-1", mock_importer)

        self.assertEqual(edges, 1)
        # Should have inserted a dependency edge
        insert_calls = [c for c in cur.execute.call_args_list
                        if "INSERT INTO dependency_edges" in str(c)]
        self.assertEqual(len(insert_calls), 1)

    def test_build_dependency_edges_force_clears(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [[], []]  # No files, no imports

        mock_importer = MagicMock()
        build_dependency_edges(conn, "proj-1", mock_importer, force=True)

        delete_call = [c for c in cur.execute.call_args_list
                       if "DELETE FROM dependency_edges" in str(c)]
        self.assertEqual(len(delete_call), 1)

    def test_build_dependency_edges_no_force(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [[], []]

        mock_importer = MagicMock()
        build_dependency_edges(conn, "proj-1", mock_importer, force=False)

        delete_calls = [c for c in cur.execute.call_args_list
                        if "DELETE" in str(c)]
        self.assertEqual(len(delete_calls), 0)

    def test_build_dependency_edges_no_unresolved_imports(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [
            [("f1", "a.rs")],
            [],  # No imports to resolve
        ]

        mock_importer = MagicMock()
        edges = build_dependency_edges(conn, "proj-1", mock_importer)

        self.assertEqual(edges, 0)

    def test_build_dependency_edges_resolution_not_found(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [
            [("f1", "a.rs"), ("f2", "b.rs")],
            [("imp1", "f1", "use external::lib;", "a.rs")],
        ]
        cur.rowcount = 0

        mock_importer = MagicMock()
        mock_importer.resolve_import.return_value = "external/lib.rs"  # Not in project

        edges = build_dependency_edges(conn, "proj-1", mock_importer)

        self.assertEqual(edges, 0)
        # When resolution fails (not in path_to_id), no UPDATE happens
        update_calls = [c for c in cur.execute.call_args_list
                        if "UPDATE file_imports" in str(c)]
        self.assertEqual(len(update_calls), 0)

    def test_build_dependency_edges_self_reference_skipped(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [
            [("f1", "a.rs")],
            [("imp1", "f1", "use self;", "a.rs")],
        ]

        mock_importer = MagicMock()
        mock_importer.resolve_import.return_value = "a.rs"  # Resolves to same file

        edges = build_dependency_edges(conn, "proj-1", mock_importer)

        self.assertEqual(edges, 0)
        insert_calls = [c for c in cur.execute.call_args_list
                        if "INSERT INTO dependency_edges" in str(c)]
        self.assertEqual(len(insert_calls), 0)

    def test_build_dependency_edges_resolve_exception(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [
            [("f1", "a.rs"), ("f2", "b.rs")],
            [("imp1", "f1", "bad import", "a.rs")],
        ]
        cur.rowcount = 0

        mock_importer = MagicMock()
        mock_importer.resolve_import.side_effect = Exception("parse error")

        # Should handle gracefully - the exception propagates since there's no try/except
        # around resolve_import in the actual code
        with self.assertRaises(Exception):
            build_dependency_edges(conn, "proj-1", mock_importer)

    def test_build_dependency_edges_multiple_edges(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [
            [("f1", "a.rs"), ("f2", "b.rs"), ("f3", "c.rs")],
            [
                ("imp1", "f1", "use crate::b;", "a.rs"),
                ("imp2", "f1", "use crate::c;", "a.rs"),
                ("imp3", "f2", "use crate::c;", "b.rs"),
            ],
        ]
        cur.rowcount = 1  # Each insert succeeds

        mock_importer = MagicMock()
        mock_importer.resolve_import.side_effect = [
            "b.rs", "c.rs", "c.rs"  # Must match paths in file_map
        ]

        edges = build_dependency_edges(conn, "proj-1", mock_importer)
        self.assertEqual(edges, 3)

    def test_build_dependency_edges_commits(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [[], []]

        mock_importer = MagicMock()
        build_dependency_edges(conn, "proj-1", mock_importer)

        conn.commit.assert_called_once()

    def test_build_dependency_edges_closes_cursor(self):
        from db import build_dependency_edges
        conn, cur = self._make_mock_conn()
        cur.fetchall.side_effect = [[], []]

        mock_importer = MagicMock()
        build_dependency_edges(conn, "proj-1", mock_importer)

        # Cursor is closed (may be called multiple times due to repository pattern)
        self.assertGreaterEqual(cur.close.call_count, 1)


# =============================================================================
# assemble_briefing Tests
# =============================================================================

class TestAssembleBriefing(unittest.TestCase):
    """Test the assemble_briefing function from brief.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_assemble_briefing_with_project_name(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        # 7 fetchone calls: project, arch, file_count, files_with_sigs, total_sigs, edge_count, import_count
        cur.fetchone.side_effect = [
            ("proj-1", "MyProject", "rust", "/path/to/project"),  # project
            None,  # no architecture
            (5, 10240),  # file count and size
            (3,),  # files with sigs
            (10,),  # total sigs
            (7,),  # edge_count
            (15,),  # import_count
        ]
        cur.fetchall.return_value = []  # No changed files, no dependents, etc.

        with patch("brief.get_connection", return_value=conn):
            result = assemble_briefing(project_name="MyProject")

        self.assertIsNotNone(result)
        self.assertIn("MyProject", result)
        self.assertIn("rust", result)

    def test_assemble_briefing_without_project_name(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("proj-1", "LatestProject", "python", "/path"),
            None,
            (2, 512),
            (1,),
            (3,),
            (0,),
            (0,),
        ]
        cur.fetchall.return_value = []

        with patch("brief.get_connection", return_value=conn):
            result = assemble_briefing()

        self.assertIsNotNone(result)
        self.assertIn("LatestProject", result)

    def test_assemble_briefing_project_not_found(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()
        cur.fetchone.return_value = None

        with patch("brief.get_connection", return_value=conn):
            # Should print "No project found" and return None
            with patch("builtins.print") as mock_print:
                result = assemble_briefing(project_name="nonexistent")
            self.assertIsNone(result)

    def test_assemble_briefing_with_architecture(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "ArchProject", "go", "/path"),
            ("Summary text here", "layer1 -> layer2"),  # architecture
            (10, 20480),
            (8,),
            (25,),
            (12,),
            (30,),
        ]
        cur.fetchall.return_value = []

        with patch("brief.get_connection", return_value=conn):
            result = assemble_briefing(project_name="ArchProject")

        self.assertIn("Summary text here", result)
        self.assertIn("layer1 -> layer2", result)

    def test_assemble_briefing_with_changed_files(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "ChangeProject", "typescript", "/path"),
            None,
            (4, 4096),
            (2,),
            (5,),
            (3,),
            (8,),
        ]

        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[("f1", "src/changed.ts")]):
            cur.fetchall.return_value = []  # No blast radius, no unchanged, no findings
            result = assemble_briefing(project_name="ChangeProject")

        self.assertIn("CHANGED", result)
        self.assertIn("src/changed.ts", result)

    def test_assemble_briefing_with_blast_radius(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "BlastProject", "rust", "/path"),
            None,
            (3, 3072),
            (2,),
            (4,),
            (2,),
            (5,),
        ]

        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[("f1", "src/lib.rs")]):
            cur.fetchall.side_effect = [
                [("main.rs",)],  # blast radius files
                [],  # unchanged with sigs
                [],  # findings
            ]
            result = assemble_briefing(project_name="BlastProject")

        self.assertIn("BLAST RADIUS", result)

    def test_assemble_briefing_with_findings(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "FindingsProject", "java", "/path"),
            None,
            (1, 100),
            (1,),
            (2,),
            (1,),
            (3,),
        ]

        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[]):
            # No changed files means: no blast radius query, unchanged query (1st fetchall), findings (2nd fetchall)
            cur.fetchall.side_effect = [
                [],  # unchanged with sigs
                [
                    ("src/Main.java", 10, 20, "high", "correctness", "Potential null pointer"),
                    ("src/Util.java", 5, 15, "medium", "security", "Unused import"),
                ],
            ]
            result = assemble_briefing(project_name="FindingsProject")

        self.assertIn("Potential null pointer", result)
        self.assertIn("HIGH", result)

    def test_assemble_briefing_no_findings(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "CleanProject", "c", "/path"),
            None,
            (1, 100),
            (1,),
            (2,),
            (0,),
            (0,),
        ]

        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[]):
            cur.fetchall.side_effect = [
                [],
                [],
                [],  # no findings
            ]
            result = assemble_briefing(project_name="CleanProject")

        self.assertIn("No open findings on record", result)

    def test_assemble_briefing_incremental_vs_full(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        # With changed files -> incremental
        cur.fetchone.side_effect = [
            ("p1", "ScopeProject", "ruby", "/path"),
            None,
            (1, 100),
            (1,),
            (2,),
            (0,),
            (0,),
        ]
        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[("f1", "a.rb")]):
            cur.fetchall.side_effect = [[], [], []]
            result = assemble_briefing(project_name="ScopeProject")
        self.assertIn("Incremental", result)

        # Without changed files -> full - need fresh mock
        conn2 = MagicMock()
        cur2 = MagicMock()
        conn2.cursor.return_value = cur2
        cur2.fetchone.side_effect = [
            ("p1", "ScopeProject", "ruby", "/path"),
            None,
            (1, 100),
            (1,),
            (2,),
            (0,),
            (0,),
        ]
        with patch("brief.get_connection", return_value=conn2), \
             patch("brief.get_changed_files", return_value=[]):
            cur2.fetchall.side_effect = [[], [], []]
            result = assemble_briefing(project_name="ScopeProject")
        self.assertIn("Full", result)

    def test_assemble_briefing_ghost_context_max_lines(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "GhostProject", "python", "/path"),
            None,
            (10, 10000),
            (8,),
            (20,),
            (5,),
            (10,),
        ]

        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[]):
            cur.fetchall.side_effect = [
                [
                    (f"file{i}.py", [{"name": f"func{i}", "type": "function"}])
                    for i in range(10)
                ],
                [],
            ]
            result = assemble_briefing(
                project_name="GhostProject",
                max_ghost_lines=5,
            )
        # Should have truncation message
        self.assertIn("more files omitted for brevity", result)

    def test_assemble_briefing_output_format_contains_sections(self):
        from brief import assemble_briefing
        conn, cur = self._make_mock_conn()

        cur.fetchone.side_effect = [
            ("p1", "FormatProject", "go", "/path"),
            None,
            (1, 100),
            (1,),
            (1,),
            (0,),
            (0,),
        ]

        with patch("brief.get_connection", return_value=conn), \
             patch("brief.get_changed_files", return_value=[]):
            cur.fetchall.side_effect = [[], [], []]
            result = assemble_briefing(project_name="FormatProject")

        self.assertIn("## Project Overview", result)
        self.assertIn("## Architecture", result)
        self.assertIn("## GHOST CONTEXT", result)
        self.assertIn("## TARGET FILES", result)
        self.assertIn("## Historical Findings", result)
        self.assertIn("## Output Contract", result)

    def test_assemble_briefing_closes_connection(self):
        from brief import assemble_briefing
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        cur.fetchone.return_value = None

        with patch("brief.get_connection", return_value=conn):
            with patch("builtins.print"):
                assemble_briefing(project_name="x")

        conn.close.assert_called_once()


# =============================================================================
# get_changed_files Tests
# =============================================================================

class TestGetChangedFiles(unittest.TestCase):
    """Test the get_changed_files function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_get_changed_files_returns_list(self):
        from db import get_changed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("f1", "a.rs"), ("f2", "b.rs")]

        result = get_changed_files(conn, "proj-1")

        self.assertEqual(result, [("f1", "a.rs"), ("f2", "b.rs")])

    def test_get_changed_files_empty(self):
        from db import get_changed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = []

        result = get_changed_files(conn, "proj-1")

        self.assertEqual(result, [])

    def test_get_changed_files_null_audited_hash(self):
        from db import get_changed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("f1", "new.rs")]

        result = get_changed_files(conn, "proj-1")

        self.assertEqual(len(result), 1)
        # Verify the query checks for NULL last_audited_hash
        call_args = cur.execute.call_args[0][0]
        self.assertIn("last_audited_hash IS NULL", call_args)

    def test_get_changed_files_closes_cursor(self):
        from db import get_changed_files
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = []

        get_changed_files(conn, "proj-1")

        cur.close.assert_called_once()


# =============================================================================
# get_or_create_project Tests
# =============================================================================

class TestGetOrCreateProject(unittest.TestCase):
    """Test the get_or_create_project function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_get_or_create_project_existing(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            ("existing-id",),  # SELECT finds existing
            None,  # UPDATE returns nothing
        ]

        pid = get_or_create_project(conn, "MyProject", "rust", "/path/to/project")

        self.assertEqual(pid, "existing-id")
        # Should have done UPDATE
        update_calls = [c for c in cur.execute.call_args_list
                        if "UPDATE projects" in str(c)]
        self.assertEqual(len(update_calls), 1)

    def test_get_or_create_project_new(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            None,  # SELECT finds nothing
            ("new-id",),  # INSERT RETURNING id
        ]

        pid = get_or_create_project(conn, "NewProject", "python", "/new/path")

        self.assertEqual(pid, "new-id")
        # Should have done INSERT
        insert_calls = [c for c in cur.execute.call_args_list
                        if "INSERT INTO projects" in str(c)]
        self.assertEqual(len(insert_calls), 1)

    def test_get_or_create_project_commits_on_create(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            None,
            ("new-id",),
        ]

        get_or_create_project(conn, "NewProject", "go", "/path")

        conn.commit.assert_called_once()

    def test_get_or_create_project_no_commit_on_existing(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            ("existing-id",),
            None,
        ]

        get_or_create_project(conn, "ExistingProject", "java", "/path")

        conn.commit.assert_not_called()

    def test_get_or_create_project_closes_cursor(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            ("existing-id",),
            None,
        ]

        get_or_create_project(conn, "P", "c", "/p")

        cur.close.assert_called_once()

    def test_get_or_create_project_updates_language(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            ("existing-id",),
            None,
        ]

        get_or_create_project(conn, "MyProject", "typescript", "/path")

        # Check UPDATE was called with the new language
        update_call = cur.execute.call_args_list[1]
        self.assertIn("typescript", update_call[0][1])

    def test_get_or_create_project_updates_timestamp(self):
        from db import get_or_create_project
        conn, cur = self._make_mock_conn()
        cur.fetchone.side_effect = [
            ("existing-id",),
            None,
        ]

        get_or_create_project(conn, "MyProject", "cpp", "/path")

        update_call = cur.execute.call_args_list[1][0][0]
        self.assertIn("updated_at = NOW()", update_call)


# =============================================================================
# fmt_sig additional edge cases
# =============================================================================

class TestFmtSigEdgeCases(unittest.TestCase):
    """Additional edge cases for fmt_sig."""

    def test_fmt_sig_none_values(self):
        from brief import fmt_sig
        sig = {"visibility": None, "signature": None, "type": None, "name": None}
        # Should not crash
        result = fmt_sig(sig)
        self.assertIsInstance(result, str)

    def test_fmt_sig_unicode(self):
        from brief import fmt_sig
        sig = {"visibility": "pub", "signature": "fn 日本語()", "type": "function", "name": "日本語"}
        self.assertEqual(fmt_sig(sig), "- pub fn 日本語()")

    def test_fmt_sig_very_long_signature(self):
        from brief import fmt_sig
        sig = {"signature": "fn " + "x" * 1000 + "()", "type": "function", "name": "long"}
        result = fmt_sig(sig)
        self.assertIn("fn", result)
        self.assertTrue(result.startswith("- "))

    def test_fmt_sig_special_characters_in_name(self):
        from brief import fmt_sig
        sig = {"type": "struct", "name": "MyStruct<T>"}
        self.assertEqual(fmt_sig(sig), "- struct MyStruct<T>")


# =============================================================================
# get_project_names Tests
# =============================================================================

class TestGetProjectNames(unittest.TestCase):
    """Test the get_project_names function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_get_project_names_returns_list(self):
        from db import get_project_names
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = [("alpha",), ("beta",), ("gamma",)]

        result = get_project_names(conn)

        self.assertEqual(result, ["alpha", "beta", "gamma"])

    def test_get_project_names_empty(self):
        from db import get_project_names
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = []

        result = get_project_names(conn)

        self.assertEqual(result, [])

    def test_get_project_names_closes_cursor(self):
        from db import get_project_names
        conn, cur = self._make_mock_conn()
        cur.fetchall.return_value = []

        get_project_names(conn)

        cur.close.assert_called_once()


# =============================================================================
# update_audit_timestamps Tests
# =============================================================================

class TestUpdateAuditTimestamps(unittest.TestCase):
    """Test the update_audit_timestamps function from db.py."""

    def _make_mock_conn(self):
        conn = MagicMock()
        cur = MagicMock()
        conn.cursor.return_value = cur
        return conn, cur

    def test_update_audit_timestamps(self):
        from db import update_audit_timestamps
        conn, cur = self._make_mock_conn()

        update_audit_timestamps(conn, "proj-1")

        conn.commit.assert_called_once()
        update_call = cur.execute.call_args[0][0]
        self.assertIn("last_audited_hash = hash", update_call)

    def test_update_audit_timestamps_closes_cursor(self):
        from db import update_audit_timestamps
        conn, cur = self._make_mock_conn()

        update_audit_timestamps(conn, "proj-1")

        cur.close.assert_called_once()


# =============================================================================
# _build_sig_cache Tests
# =============================================================================

class TestBuildSigCache(unittest.TestCase):
    """Test the _build_sig_cache helper from core.py."""

    def test_build_sig_cache_basic(self):
        from core import _build_sig_cache
        sigs = [
            {"type": "function", "name": "foo", "visibility": "pub", "signature": "fn foo()"},
            {"type": "struct", "name": "Bar", "visibility": ""},
        ]
        result = _build_sig_cache(sigs)
        self.assertEqual(len(result), 2)
        self.assertEqual(result[0]["type"], "function")
        self.assertEqual(result[0]["signature"], "fn foo()")
        self.assertEqual(result[1]["type"], "struct")

    def test_build_sig_cache_empty(self):
        from core import _build_sig_cache
        result = _build_sig_cache([])
        self.assertEqual(result, [])

    def test_build_sig_cache_with_generic_params(self):
        from core import _build_sig_cache
        sigs = [{"type": "fn", "name": "map", "generic_params": ["T", "U"]}]
        result = _build_sig_cache(sigs)
        self.assertEqual(result[0]["generic_params"], ["T", "U"])

    def test_build_sig_cache_missing_fields(self):
        from core import _build_sig_cache
        sigs = [{"other": "stuff"}]
        result = _build_sig_cache(sigs)
        self.assertEqual(result[0]["type"], "unknown")
        self.assertEqual(result[0]["name"], "")
        self.assertEqual(result[0]["visibility"], "")
        self.assertEqual(result[0]["signature"], "")


# =============================================================================
# _build_import_records Tests
# =============================================================================

class TestBuildImportRecords(unittest.TestCase):
    """Test the _build_import_records helper from core.py."""

    def test_build_import_records_basic(self):
        from core import _build_import_records
        imps = [
            {"import_text": "use std::io;", "import_type": "internal", "line_start": 1, "line_end": 1},
        ]
        result = _build_import_records(imps)
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]["import_text"], "use std::io;")
        self.assertEqual(result[0]["import_type"], "internal")
        self.assertEqual(result[0]["line_start"], 1)
        self.assertEqual(result[0]["line_end"], 1)

    def test_build_import_records_empty(self):
        from core import _build_import_records
        result = _build_import_records([])
        self.assertEqual(result, [])

    def test_build_import_records_defaults(self):
        from core import _build_import_records
        imps = [{"import_text": "import x"}]
        result = _build_import_records(imps)
        self.assertEqual(result[0]["import_type"], "internal")
        self.assertIsNone(result[0]["resolved_path"])
        self.assertIsNone(result[0]["line_start"])
        self.assertIsNone(result[0]["line_end"])


if __name__ == "__main__":
    unittest.main()
