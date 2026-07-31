"""End-to-end tests for the SQLite backend through the real MCP call_tool
dispatcher and connection-selection layer — not just the repository layer.
"""

import json
import os
import sqlite3
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Any
from unittest.mock import patch

import app.config
import app.pool
from app.activation import enable_project
from app.pool import get_connection_for_project
from app.server import HOST_TOKEN_ARGUMENT, PROJECT_ROOT_ARGUMENT, call_tool
from mcp.types import TextContent

_TEST_TOKEN = "s3cr3t"  # noqa: S105 - a test fixture value, not a real secret


def _force_unconfigured() -> None:
    """Make Config.is_configured False regardless of the real environment."""
    for var in ("PGHOST", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD", "SLUGAUDIT_CONFIG"):
        os.environ.pop(var, None)
    app.config._config = None
    app.config._find_config = lambda: None


class _SqliteBackendTestCase(unittest.IsolatedAsyncioTestCase):
    """Base class that pins the process to the SQLite backend for its tests.

    Also authenticates _project_root for the duration of each test: with no
    unauthenticated fallback (see app/server.py), every call in this file
    needs a matching _host_token to actually target its TemporaryDirectory
    rather than silently falling back to the real process cwd.
    """

    def setUp(self) -> None:
        self._saved_env = {
            var: os.environ.get(var)
            for var in ("PGHOST", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD", "SLUGAUDIT_CONFIG")
        }
        self._original_find_config = app.config._find_config
        _force_unconfigured()
        self.enterContext(
            patch("app.server._host_token_configured", return_value=_TEST_TOKEN)
        )

    def tearDown(self) -> None:
        for var, val in self._saved_env.items():
            if val is not None:
                os.environ[var] = val
            else:
                os.environ.pop(var, None)
        app.config._find_config = self._original_find_config
        app.config._config = None

    async def _call(self, name: str, args: dict[str, Any]) -> list[TextContent]:
        """call_tool with the test host token merged in automatically."""
        return await call_tool(name, {**args, HOST_TOKEN_ARGUMENT: _TEST_TOKEN})


class TestGetConnectionForProjectDispatch(_SqliteBackendTestCase):
    async def test_selects_sqlite_when_postgres_not_configured(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".planning" / "slugaudit").mkdir(parents=True)
            async with get_connection_for_project(str(root)) as conn:
                self.assertIsInstance(conn, sqlite3.Connection)
            self.assertTrue((root / ".planning" / "slugaudit" / "audit.db").exists())

    async def test_requires_activation_directory_to_already_exist(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)  # never activated
            with self.assertRaisesRegex(RuntimeError, "not enabled"):
                async with get_connection_for_project(str(root)):
                    pass


class TestFullToolStackAgainstSqlite(_SqliteBackendTestCase):
    async def test_overview_search_read_dependents_brief_finding(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "app.py").write_text(
                "def risky():\n    return eval('1+1')\n", encoding="utf-8"
            )
            enable_project(root)

            overview = await self._call("audit_overview", {PROJECT_ROOT_ARGUMENT: str(root)})
            self.assertIn("**Files:** 1", overview[0].text)

            search = await self._call(
                "audit_search", {PROJECT_ROOT_ARGUMENT: str(root), "pattern": "eval"}
            )
            self.assertIn("app.py", search[0].text)

            read = await self._call(
                "audit_read_file",
                {PROJECT_ROOT_ARGUMENT: str(root), "paths": ["app.py"]},
            )
            self.assertIn("def risky", read[0].text)

            brief = await self._call("audit_brief", {PROJECT_ROOT_ARGUMENT: str(root)})
            payload = json.loads(brief[0].text)
            self.assertEqual(payload["risk_leads"][0]["path"], "app.py")
            self.assertEqual(payload["risk_leads"][0]["patterns"][0]["type"], "eval")

            finding = await self._call(
                "audit_finding",
                {
                    PROJECT_ROOT_ARGUMENT: str(root),
                    "path": "app.py",
                    "line_start": 2,
                    "severity": "high",
                    "category": "security",
                    "title": "eval used",
                    "description": "Dangerous eval call.",
                },
            )
            self.assertIn('"created":true', finding[0].text)

            brief2 = await self._call("audit_brief", {PROJECT_ROOT_ARGUMENT: str(root)})
            payload2 = json.loads(brief2[0].text)
            self.assertEqual(len(payload2["open_findings"]), 1)
            self.assertEqual(payload2["open_findings"][0]["severity"], "high")

    async def test_dependents_tool_runs_end_to_end(self) -> None:
        # Dependency *resolution* correctness (does `from b import x` resolve
        # to b.py) is a languages/python.py concern, already covered by
        # tests/test_sqlite_backend.py's stub-resolver test for the storage
        # layer itself. This only proves the audit_dependents tool call
        # reaches the SQLite-backed query path and returns cleanly either way.
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("x = 1\n", encoding="utf-8")
            (root / "b.py").write_text("y = 2\n", encoding="utf-8")
            enable_project(root)

            await self._call("audit_overview", {PROJECT_ROOT_ARGUMENT: str(root)})
            result = await self._call(
                "audit_dependents",
                {PROJECT_ROOT_ARGUMENT: str(root), "file_path": "b.py"},
            )
            self.assertIn("b.py", result[0].text)

    async def test_raw_sql_returns_clear_message_instead_of_syntax_error(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("x = 1\n", encoding="utf-8")
            enable_project(root)
            await self._call("audit_overview", {PROJECT_ROOT_ARGUMENT: str(root)})

            result = await self._call(
                "audit_raw_sql",
                {PROJECT_ROOT_ARGUMENT: str(root), "query": "SELECT path FROM files"},
            )
            self.assertIn("requires PostgreSQL", result[0].text)
            self.assertNotIn("syntax error", result[0].text)

    async def test_off_purges_local_sqlite_database(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("x = 1\n", encoding="utf-8")
            enable_project(root)
            await self._call("audit_overview", {PROJECT_ROOT_ARGUMENT: str(root)})
            self.assertTrue((root / ".planning" / "slugaudit" / "audit.db").exists())

            from app.server import PROJECT_CONTROL_TOOL

            result = await self._call(
                PROJECT_CONTROL_TOOL, {PROJECT_ROOT_ARGUMENT: str(root), "action": "off"}
            )
            self.assertTrue(json.loads(result[0].text)["slugaudit_control"]["changed"])
            self.assertFalse((root / ".planning" / "slugaudit").exists())

    async def test_off_on_never_activated_project_is_a_graceful_noop(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)  # never enabled
            from app.server import PROJECT_CONTROL_TOOL

            result = await self._call(
                PROJECT_CONTROL_TOOL, {PROJECT_ROOT_ARGUMENT: str(root), "action": "off"}
            )
            payload = json.loads(result[0].text)["slugaudit_control"]
            self.assertFalse(payload["changed"])


if __name__ == "__main__":
    unittest.main()
