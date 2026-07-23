"""Tests for the host-facing /slugaudit on|off adapter API."""

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

from app.activation import disable_project, enable_project


class TestActivation(unittest.TestCase):
    def test_on_creates_only_the_trigger(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            activation = enable_project(root)
            self.assertTrue(activation.is_dir())
            self.assertEqual(list(activation.iterdir()), [])

    def test_off_purges_before_removing_trigger(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            activation = enable_project(root)
            with patch(
                "app.activation.ProjectRepository.purge_by_path", return_value=True
            ) as purge:
                disabled = disable_project(root, object())
            self.assertTrue(disabled)
            self.assertFalse(activation.exists())
            purge.assert_called_once_with(str(root.resolve()))

    def test_off_retains_trigger_when_database_purge_fails(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            activation = enable_project(root)
            with (
                patch(
                    "app.activation.ProjectRepository.purge_by_path",
                    side_effect=RuntimeError("database unavailable"),
                ),
                self.assertRaises(RuntimeError),
            ):
                disable_project(root, object())
            self.assertTrue(activation.is_dir())

    def test_off_is_idempotent_when_disabled(self) -> None:
        with TemporaryDirectory() as temporary:
            self.assertFalse(disable_project(temporary, object()))

    def test_on_rejects_symlinked_planning_directory(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "elsewhere"
            target.mkdir()
            (root / ".planning").symlink_to(target, target_is_directory=True)

            with self.assertRaisesRegex(ValueError, "symlinked .planning"):
                enable_project(root)


if __name__ == "__main__":
    unittest.main()
