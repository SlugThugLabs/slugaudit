"""Tests for project state tracking."""

import json
import os
import tempfile
import unittest
from typing import Any

from app.manifest import PARSER_VERSION
from app.state import SCHEMA_VERSION, ProjectState, load_state, save_state


def _state_data(project_path: str) -> dict[str, Any]:
    return {
        "contract_version": 1,
        "schema_version": SCHEMA_VERSION,
        "project_path": project_path,
        "project_name": "test-project",
        "project_id": "uuid-123",
        "revision_id": "revision-123",
        "manifest_hash": "manifest-123",
        "parser_version": PARSER_VERSION,
        "last_synced_at": "2026-01-01T00:00:00+00:00",
        "file_count": 1,
        "signature_count": 100,
        "languages": ["python"],
        "files": {
            "src/main.py": {
                "hash": "file-hash",
                "size": 12,
                "language": "python",
            }
        },
    }


class TestProjectState(unittest.TestCase):
    """ProjectState data class and file persistence."""

    def test_default_creation(self) -> None:
        state = ProjectState()
        self.assertEqual(state.project_name, "")
        self.assertEqual(state.file_count, 0)
        self.assertEqual(state.signature_count, 0)
        self.assertEqual(state.language, "unknown")

    def test_from_dict(self) -> None:
        data = _state_data("/test/path")
        state = ProjectState.from_dict(data)
        self.assertEqual(state.project_name, "test-project")
        self.assertEqual(state.project_id, "uuid-123")
        self.assertEqual(state.revision_id, "revision-123")
        self.assertEqual(state.manifest_hash, "manifest-123")
        self.assertEqual(state.file_count, 1)
        self.assertEqual(state.signature_count, 100)
        self.assertEqual(state.language, "python")

    def test_to_dict_roundtrip(self) -> None:
        original = ProjectState.from_dict(_state_data("/test"))
        data = original.to_dict()
        restored = ProjectState.from_dict(data)
        self.assertEqual(original.project_name, restored.project_name)
        self.assertEqual(original.project_id, restored.project_id)
        self.assertEqual(original.revision_id, restored.revision_id)
        self.assertEqual(original.manifest_hash, restored.manifest_hash)
        self.assertEqual(original.file_count, restored.file_count)
        self.assertEqual(original.signature_count, restored.signature_count)
        self.assertEqual(original.files, restored.files)

    def test_from_dict_rejects_missing_keys(self) -> None:
        with self.assertRaisesRegex(ValueError, "missing required fields"):
            ProjectState.from_dict({})

    def test_save_and_load_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            state_directory = os.path.join(tmpdir, ".planning", "slugaudit")
            os.makedirs(state_directory)
            data = _state_data(os.path.realpath(tmpdir))
            data["project_name"] = "test"
            data["project_id"] = "id-456"
            data["languages"] = ["go"]
            data["files"]["src/main.py"]["language"] = "go"
            orig = ProjectState.from_dict(data)
            save_state(tmpdir, orig)

            loaded = load_state(tmpdir)
            self.assertIsNotNone(loaded)
            if loaded is None:
                self.fail("saved state did not load")
            self.assertEqual(loaded.project_name, "test")
            self.assertEqual(loaded.project_id, "id-456")
            self.assertEqual(loaded.file_count, 1)
            self.assertEqual(loaded.language, "go")

    def test_load_state_returns_none_when_no_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            state = load_state(tmpdir)
            self.assertIsNone(state)

    def test_load_state_handles_corrupt_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            state_dir = os.path.join(tmpdir, ".planning", "slugaudit")
            os.makedirs(state_dir)
            with open(os.path.join(state_dir, "state.json"), "w") as f:
                f.write("not valid json")

            state = load_state(tmpdir)
            self.assertIsNone(state)

    def test_state_dir_returns_correct_path(self) -> None:
        from app.state import state_dir
        p = state_dir("/some/project")
        self.assertEqual(str(p), os.path.join("/some/project", ".planning", "slugaudit"))

    def test_save_state_does_not_create_activation_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            state = ProjectState(project_path=tmpdir, project_name="test")
            with self.assertRaisesRegex(RuntimeError, "refusing to recreate"):
                save_state(tmpdir, state)
            self.assertFalse(os.path.exists(os.path.join(tmpdir, ".planning")))

    def test_load_state_rejects_state_for_different_project_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            state_directory = os.path.join(tmpdir, ".planning", "slugaudit")
            os.makedirs(state_directory)
            with open(os.path.join(state_directory, "state.json"), "w") as state_file:
                json.dump(_state_data("/a/different/project"), state_file)

            self.assertIsNone(load_state(tmpdir))


if __name__ == "__main__":
    unittest.main()
