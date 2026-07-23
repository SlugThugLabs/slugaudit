"""Tests for the installed MCP-facing runtime contract."""

import unittest

from app.instructions import MCP_INSTRUCTIONS
from app.server import SERVER, _freshness_content
from app.state import SCHEMA_VERSION


class TestInitializationContract(unittest.TestCase):
    def test_initialization_exposes_ai_first_instructions(self) -> None:
        options = SERVER.create_initialization_options()
        self.assertEqual(options.instructions, MCP_INSTRUCTIONS)
        self.assertIn("evidence index for AI auditors", options.instructions)
        self.assertIn(".planning/slugaudit/", options.instructions)
        self.assertIn("there are no\nmanual sync", options.instructions)

    def test_instructions_do_not_advertise_removed_tools(self) -> None:
        for removed in ("audit_changed", "audit_status", "audit_source", "audit_init_db"):
            self.assertNotIn(removed, MCP_INSTRUCTIONS)

    def test_freshness_metadata_is_machine_readable_and_complete(self) -> None:
        import json
        from types import SimpleNamespace

        state = SimpleNamespace(
            contract_version=1,
            schema_version=SCHEMA_VERSION,
            project_id="project-1",
            revision_id="revision-2",
            manifest_hash="abc123",
            last_synced_at="2026-07-22T12:00:00+00:00",
        )
        payload = json.loads(_freshness_content(state).text)["slugaudit_meta"]
        self.assertEqual(payload["contract_version"], 1)
        self.assertEqual(payload["schema_version"], SCHEMA_VERSION)
        self.assertEqual(payload["freshness"], "verified")
        self.assertEqual(payload["revision_id"], "revision-2")

    def test_freshness_metadata_refuses_incomplete_state(self) -> None:
        from types import SimpleNamespace

        state = SimpleNamespace(
            contract_version=1,
            schema_version=SCHEMA_VERSION,
            project_id="project-1",
            revision_id="",
            manifest_hash="abc123",
            last_synced_at="2026-07-22T12:00:00+00:00",
        )
        with self.assertRaises(RuntimeError):
            _freshness_content(state)


if __name__ == "__main__":
    unittest.main()
