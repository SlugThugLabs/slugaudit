"""Focused tests for the schema-hidden native host adapter protocol."""

from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from typing import Any, cast
import unittest
from unittest.mock import patch

from mcp.types import CallToolRequest, CallToolRequestParams, CallToolResult, TextContent

from app.server import (
    HOST_TOKEN_ARGUMENT,
    PROJECT_CONTROL_TOOL,
    PROJECT_ROOT_ARGUMENT,
    SERVER,
    call_tool,
    list_tools,
)
from app.state import SCHEMA_VERSION


def _published_state() -> SimpleNamespace:
    return SimpleNamespace(
        contract_version=1,
        schema_version=SCHEMA_VERSION,
        project_id="project-1",
        revision_id="revision-2",
        manifest_hash="abc123",
        last_synced_at="2026-07-23T12:00:00+00:00",
    )


@asynccontextmanager
async def _connection(connection: Any = None) -> AsyncIterator[Any]:
    yield connection if connection is not None else object()


class TestServerAdapterProtocol(unittest.IsolatedAsyncioTestCase):
    async def test_reserved_project_root_overrides_cwd_and_is_not_sent_to_handler(
        self,
    ) -> None:
        captured: dict[str, Any] = {}

        @asynccontextmanager
        async def synchronized(project_root: str, conn: Any) -> AsyncIterator[Any]:
            captured["project_root"] = project_root
            captured["connection"] = conn
            yield _published_state()

        async def handler(
            conn: Any, state: Any, arguments: dict[str, Any]
        ) -> list[TextContent]:
            captured["handler_arguments"] = arguments
            return [TextContent(type="text", text="overview")]

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                patch("app.server.os.getcwd", return_value="/wrong/project"),
                patch("app.server.get_db", return_value=_connection("database")),
                patch("app.server.synchronized_project", side_effect=synchronized),
                patch.dict("app.server.HANDLERS", {"audit_overview": handler}),
                patch("app.server._host_token_configured", return_value=None),
            ):
                result = await call_tool(
                    "audit_overview",
                    {PROJECT_ROOT_ARGUMENT: str(root), "detail": "compact"},
                )

        self.assertEqual(captured["project_root"], str(root.resolve()))
        self.assertEqual(captured["connection"], "database")
        self.assertEqual(captured["handler_arguments"], {"detail": "compact"})
        self.assertEqual(result[0].text, "overview")

    async def test_matching_host_token_authorizes_project_root_override(self) -> None:
        captured: dict[str, Any] = {}

        @asynccontextmanager
        async def synchronized(project_root: str, conn: Any) -> AsyncIterator[Any]:
            captured["project_root"] = project_root
            yield _published_state()

        async def handler(
            conn: Any, state: Any, arguments: dict[str, Any]
        ) -> list[TextContent]:
            captured["handler_arguments"] = arguments
            return [TextContent(type="text", text="overview")]

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                patch("app.server.os.getcwd", return_value="/wrong/project"),
                patch("app.server.get_db", return_value=_connection("database")),
                patch("app.server.synchronized_project", side_effect=synchronized),
                patch.dict("app.server.HANDLERS", {"audit_overview": handler}),
                patch("app.server._host_token_configured", return_value="s3cr3t"),
            ):
                result = await call_tool(
                    "audit_overview",
                    {
                        PROJECT_ROOT_ARGUMENT: str(root),
                        HOST_TOKEN_ARGUMENT: "s3cr3t",
                    },
                )

        self.assertEqual(captured["project_root"], str(root.resolve()))
        self.assertEqual(captured["handler_arguments"], {})
        self.assertEqual(result[0].text, "overview")

    async def _assert_project_root_falls_back_to_cwd(
        self, supplied_arguments: dict[str, Any]
    ) -> None:
        captured: dict[str, Any] = {}

        @asynccontextmanager
        async def synchronized(project_root: str, conn: Any) -> AsyncIterator[Any]:
            captured["project_root"] = project_root
            yield _published_state()

        async def handler(
            conn: Any, state: Any, arguments: dict[str, Any]
        ) -> list[TextContent]:
            return [TextContent(type="text", text="overview")]

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                patch("app.server.os.getcwd", return_value="/actual/cwd"),
                patch("app.server.get_db", return_value=_connection()),
                patch("app.server.synchronized_project", side_effect=synchronized),
                patch.dict("app.server.HANDLERS", {"audit_overview": handler}),
                patch("app.server._host_token_configured", return_value="s3cr3t"),
            ):
                await call_tool(
                    "audit_overview",
                    {PROJECT_ROOT_ARGUMENT: str(root), **supplied_arguments},
                )

        self.assertEqual(captured["project_root"], "/actual/cwd")

    async def test_missing_or_wrong_host_token_falls_back_to_cwd(self) -> None:
        for supplied_arguments in (
            {},
            {HOST_TOKEN_ARGUMENT: "wrong-token"},
        ):
            with self.subTest(arguments=supplied_arguments):
                await self._assert_project_root_falls_back_to_cwd(supplied_arguments)

    async def test_control_route_and_reserved_arguments_are_not_advertised(self) -> None:
        tools = await list_tools()
        self.assertNotIn(PROJECT_CONTROL_TOOL, {tool.name for tool in tools})
        for tool in tools:
            self.assertEqual(tool.inputSchema.get("additionalProperties"), False)
            properties = tool.inputSchema.get("properties", {})
            self.assertNotIn(PROJECT_ROOT_ARGUMENT, properties)
            self.assertNotIn(HOST_TOKEN_ARGUMENT, properties)

    async def test_on_creates_trigger_without_opening_database(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch(
                "app.server.get_db",
                side_effect=AssertionError("on must not open the database"),
            ):
                result = await call_tool(
                    PROJECT_CONTROL_TOOL,
                    {"action": "on", PROJECT_ROOT_ARGUMENT: str(root)},
                )

            payload = json.loads(result[0].text)["slugaudit_control"]
            self.assertTrue((root / ".planning" / "slugaudit").is_dir())
            self.assertEqual(payload["action"], "on")
            self.assertTrue(payload["changed"])

            with patch(
                "app.server.get_db",
                side_effect=AssertionError("on must not open the database"),
            ):
                repeated = await call_tool(
                    PROJECT_CONTROL_TOOL,
                    {"action": "on", PROJECT_ROOT_ARGUMENT: str(root)},
                )
            repeated_payload = json.loads(repeated[0].text)["slugaudit_control"]
            self.assertFalse(repeated_payload["changed"])

    async def test_off_purges_and_removes_trigger(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            activation = root / ".planning" / "slugaudit"
            activation.mkdir(parents=True)

            with (
                patch("app.server.get_db", return_value=_connection("database")),
                patch(
                    "repositories.ProjectRepository.purge_by_path",
                    return_value=True,
                ) as purge,
            ):
                result = await call_tool(
                    PROJECT_CONTROL_TOOL,
                    {"action": "off", PROJECT_ROOT_ARGUMENT: str(root)},
                )

            payload = json.loads(result[0].text)["slugaudit_control"]
            self.assertFalse(activation.exists())
            self.assertEqual(payload["action"], "off")
            self.assertTrue(payload["changed"])
            purge.assert_called_once_with(str(root.resolve()))

    async def test_off_retains_trigger_when_database_purge_fails(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            activation = root / ".planning" / "slugaudit"
            activation.mkdir(parents=True)

            with (
                patch("app.server.get_db", return_value=_connection()),
                patch(
                    "repositories.ProjectRepository.purge_by_path",
                    side_effect=RuntimeError("database unavailable"),
                ),
            ):
                request = CallToolRequest(
                    params=CallToolRequestParams(
                        name=PROJECT_CONTROL_TOOL,
                        arguments={
                            "action": "off",
                            PROJECT_ROOT_ARGUMENT: str(root),
                        },
                    )
                )
                result = await SERVER.request_handlers[CallToolRequest](request)

            self.assertTrue(activation.is_dir())
            self.assertIsInstance(result.root, CallToolResult)
            call_result = cast(CallToolResult, result.root)
            self.assertTrue(call_result.isError)
            self.assertIsInstance(call_result.content[0], TextContent)
            error_content = cast(TextContent, call_result.content[0])
            self.assertIn("database unavailable", error_content.text)

    async def test_public_result_includes_verified_revision_metadata(self) -> None:
        @asynccontextmanager
        async def synchronized(project_root: str, conn: Any) -> AsyncIterator[Any]:
            yield _published_state()

        async def handler(
            conn: Any, state: Any, arguments: dict[str, Any]
        ) -> list[TextContent]:
            return [TextContent(type="text", text="evidence")]

        with (
            patch("app.server.get_db", return_value=_connection()),
            patch("app.server.synchronized_project", side_effect=synchronized),
            patch.dict("app.server.HANDLERS", {"audit_overview": handler}),
        ):
            result = await call_tool("audit_overview", {})

        metadata = json.loads(result[-1].text)["slugaudit_meta"]
        self.assertEqual(metadata["freshness"], "verified")
        self.assertEqual(metadata["revision_id"], "revision-2")
        self.assertEqual(result[0].text, "evidence")

    async def test_incomplete_public_freshness_is_a_protocol_failure(self) -> None:
        state = _published_state()
        state.revision_id = ""

        @asynccontextmanager
        async def synchronized(project_root: str, conn: Any) -> AsyncIterator[Any]:
            yield state

        async def handler(
            conn: Any, state: Any, arguments: dict[str, Any]
        ) -> list[TextContent]:
            return [TextContent(type="text", text="must not escape")]

        with (
            patch("app.server.get_db", return_value=_connection()),
            patch("app.server.synchronized_project", side_effect=synchronized),
            patch.dict("app.server.HANDLERS", {"audit_overview": handler}),
        ):
            request = CallToolRequest(
                params=CallToolRequestParams(name="audit_overview", arguments={})
            )
            result = await SERVER.request_handlers[CallToolRequest](request)

        self.assertIsInstance(result.root, CallToolResult)
        call_result = cast(CallToolResult, result.root)
        self.assertTrue(call_result.isError)
        self.assertIsInstance(call_result.content[0], TextContent)
        error_content = cast(TextContent, call_result.content[0])
        self.assertIn("freshness metadata", error_content.text)


if __name__ == "__main__":
    unittest.main()
