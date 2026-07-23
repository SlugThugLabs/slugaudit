"""Regression tests for the filesystem-to-database freshness contract.

These tests intentionally exercise the public state/sync boundary.  An MCP
query may only consume a ``ProjectState`` after the complete on-disk source
manifest has been reconciled with the database revision represented by that
state.
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path
from types import SimpleNamespace
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import MagicMock, patch

from app.manifest import PARSER_VERSION, build_manifest
from app.state import (
    SCHEMA_VERSION,
    ProjectState,
    find_project_root,
    load_state,
    save_state,
)
from app.sync import _sync_locked, ensure_synced, synchronized_project


def _activate(project: Path) -> Path:
    trigger = project / ".planning" / "slugaudit"
    trigger.mkdir(parents=True)
    return trigger


def _state(project: Path) -> ProjectState:
    return ProjectState(
        contract_version=1,
        schema_version=SCHEMA_VERSION,
        project_path=str(project),
        project_name=project.name,
        project_id="project-1",
        revision_id="revision-1",
        manifest_hash="manifest-1",
        parser_version=PARSER_VERSION,
        last_synced_at="2026-07-22T00:00:00+00:00",
        file_count=1,
        signature_count=1,
        languages=["python"],
        files={
            "src/main.py": {
                "hash": "file-hash-1",
                "size": 13,
                "language": "python",
            }
        },
    )


def _write(project: Path, relpath: str, content: str) -> None:
    target = project / relpath
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def _published_state(
    project: Path,
    revision_id: str = "revision-old",
) -> ProjectState:
    manifest = build_manifest(project)
    return ProjectState.from_sync_result(
        project_path=str(project),
        project_id="project-1",
        revision_id=revision_id,
        manifest=manifest,
        signature_count=1,
        synced_at="2026-07-22T00:00:00+00:00",
    )


def _reconcile_result(revision_id: str = "revision-new") -> SimpleNamespace:
    return SimpleNamespace(
        project_id="project-1",
        revision_id=revision_id,
        signatures_extracted=2,
    )


class TestFilesystemManifest(unittest.TestCase):
    """Every query hashes the complete supported source set on disk."""

    def test_manifest_reconciles_unchanged_modified_added_and_deleted_files(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "from helper import answer\n")
            _write(project, "src/helper.py", "answer = 41\n")

            initial = build_manifest(project)
            self.assertEqual(set(initial.files), {"src/helper.py", "src/main.py"})
            self.assertEqual(initial.languages, ("python",))

            unchanged = build_manifest(project)
            self.assertEqual(unchanged, initial)

            _write(project, "src/helper.py", "answer = 42\n")
            modified = build_manifest(project)
            self.assertNotEqual(modified.manifest_hash, initial.manifest_hash)
            self.assertNotEqual(
                modified.files["src/helper.py"].hash,
                initial.files["src/helper.py"].hash,
            )
            self.assertEqual(
                modified.files["src/main.py"].hash,
                initial.files["src/main.py"].hash,
            )

            _write(project, "src/lib.rs", "pub fn answer() -> i32 { 42 }\n")
            added = build_manifest(project)
            self.assertEqual(
                set(added.files), {"src/helper.py", "src/lib.rs", "src/main.py"}
            )
            self.assertEqual(added.languages, ("python", "rust"))
            self.assertNotEqual(added.manifest_hash, modified.manifest_hash)

            (project / "src" / "main.py").unlink()
            deleted = build_manifest(project)
            self.assertEqual(set(deleted.files), {"src/helper.py", "src/lib.rs"})
            self.assertNotIn("src/main.py", deleted.files)
            self.assertNotEqual(deleted.manifest_hash, added.manifest_hash)

    def test_manifest_excludes_state_generated_vendor_and_unsupported_files(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            trigger = _activate(project)
            _write(project, "app/main.py", "print('indexed')\n")
            _write(project, "vendor/copied.py", "print('not indexed')\n")
            _write(project, "node_modules/pkg/index.ts", "export const no = true\n")
            _write(project, "README.md", "not source\n")
            (trigger / "state.json").write_text("{}", encoding="utf-8")

            manifest = build_manifest(project)

            self.assertEqual(set(manifest.files), {"app/main.py"})


class TestReconciliationGate(unittest.TestCase):
    """A state is returned only after disk, DB, and revision agree."""

    def test_initial_use_imports_complete_manifest_and_publishes_state(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "print('initial')\n")
            conn = object()

            with patch(
                "app.sync.reconcile_project", return_value=_reconcile_result()
            ) as reconcile:
                state = _sync_locked(project.resolve(), conn)

            manifest = build_manifest(project)
            reconcile.assert_called_once_with(
                str(project.resolve()),
                manifest,
                conn=conn,
                force_full=True,
            )
            self.assertEqual(state.revision_id, "revision-new")
            self.assertEqual(state.manifest_hash, manifest.manifest_hash)
            self.assertEqual(set(state.files), {"src/main.py"})
            self.assertEqual(load_state(project), state)

    def test_unchanged_manifest_uses_verified_state_without_reimport(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "print('unchanged')\n")
            expected = _published_state(project)
            save_state(project, expected)

            with (
                patch("app.sync._database_matches_state", return_value=True),
                patch("app.sync.reconcile_project") as reconcile,
            ):
                conn = MagicMock()
                state = _sync_locked(project.resolve(), conn)

            self.assertEqual(state, expected)
            reconcile.assert_not_called()
            conn.rollback.assert_called_once_with()

    def test_modified_file_forces_incremental_reconciliation_and_replacement(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "value = 1\n")
            old_state = _published_state(project)
            save_state(project, old_state)
            _write(project, "src/main.py", "value = 2\n")
            conn = object()

            with (
                patch("app.sync._database_matches_state", return_value=True),
                patch(
                    "app.sync.reconcile_project",
                    return_value=_reconcile_result(),
                ) as reconcile,
            ):
                state = _sync_locked(project.resolve(), conn)

            manifest = build_manifest(project)
            reconcile.assert_called_once_with(
                str(project.resolve()), manifest, conn=conn, force_full=False
            )
            self.assertNotEqual(
                state.files["src/main.py"]["hash"],
                old_state.files["src/main.py"]["hash"],
            )
            self.assertEqual(state.manifest_hash, manifest.manifest_hash)

    def test_added_file_is_included_in_incremental_reconciliation(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "print('existing')\n")
            save_state(project, _published_state(project))
            _write(project, "src/new.py", "print('new')\n")
            conn = object()

            with (
                patch("app.sync._database_matches_state", return_value=True),
                patch(
                    "app.sync.reconcile_project",
                    return_value=_reconcile_result(),
                ) as reconcile,
            ):
                state = _sync_locked(project.resolve(), conn)

            manifest = build_manifest(project)
            reconcile.assert_called_once_with(
                str(project.resolve()), manifest, conn=conn, force_full=False
            )
            self.assertEqual(set(state.files), {"src/main.py", "src/new.py"})

    def test_deleted_file_is_absent_from_reconciliation_and_published_state(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/keep.py", "keep = True\n")
            _write(project, "src/remove.py", "remove = True\n")
            save_state(project, _published_state(project))
            (project / "src" / "remove.py").unlink()
            conn = object()

            with (
                patch("app.sync._database_matches_state", return_value=True),
                patch(
                    "app.sync.reconcile_project",
                    return_value=_reconcile_result(),
                ) as reconcile,
            ):
                state = _sync_locked(project.resolve(), conn)

            manifest = build_manifest(project)
            reconcile.assert_called_once_with(
                str(project.resolve()), manifest, conn=conn, force_full=False
            )
            self.assertEqual(set(state.files), {"src/keep.py"})
            self.assertNotIn("src/remove.py", load_state(project).files)  # type: ignore[union-attr]

    def test_corrupt_state_triggers_full_rebuild(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            trigger = _activate(project)
            _write(project, "src/main.py", "print('recover')\n")
            (trigger / "state.json").write_text("not-json", encoding="utf-8")
            conn = object()

            with patch(
                "app.sync.reconcile_project", return_value=_reconcile_result()
            ) as reconcile:
                _sync_locked(project.resolve(), conn)

            manifest = build_manifest(project)
            reconcile.assert_called_once_with(
                str(project.resolve()), manifest, conn=conn, force_full=True
            )

    def test_database_revision_mismatch_triggers_full_rebuild(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "print('recover db')\n")
            save_state(project, _published_state(project))
            conn = object()

            with (
                patch("app.sync._database_matches_state", return_value=False),
                patch(
                    "app.sync.reconcile_project",
                    return_value=_reconcile_result(),
                ) as reconcile,
            ):
                _sync_locked(project.resolve(), conn)

            manifest = build_manifest(project)
            reconcile.assert_called_once_with(
                str(project.resolve()), manifest, conn=conn, force_full=True
            )


class TestNoStaleFallback(unittest.IsolatedAsyncioTestCase):
    async def test_sync_failure_aborts_query_instead_of_returning_old_state(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            _write(project, "src/main.py", "print('stale')\n")
            save_state(project, _published_state(project))

            with patch(
                "app.sync._sync_locked",
                side_effect=RuntimeError("database unavailable"),
            ):
                with self.assertRaisesRegex(
                    RuntimeError,
                    "SlugAudit freshness check failed: database unavailable",
                ):
                    await ensure_synced(str(project), conn=object())

    async def test_query_holds_project_lock_until_evidence_is_consumed(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            first_entered = asyncio.Event()
            release_first = asyncio.Event()
            second_entered = asyncio.Event()
            conn_one = MagicMock()
            conn_two = MagicMock()

            async def first_query() -> None:
                async with synchronized_project(str(project), conn_one):
                    first_entered.set()
                    await release_first.wait()

            async def second_query() -> None:
                async with synchronized_project(str(project), conn_two):
                    second_entered.set()

            with patch("app.sync._sync_locked", return_value=_state(project)):
                first = asyncio.create_task(first_query())
                await asyncio.wait_for(first_entered.wait(), timeout=1)
                second = asyncio.create_task(second_query())
                await asyncio.sleep(0.05)
                self.assertFalse(second_entered.is_set())
                release_first.set()
                await asyncio.wait_for(asyncio.gather(first, second), timeout=1)

            self.assertTrue(second_entered.is_set())
            conn_one.rollback.assert_called_once_with()
            conn_two.rollback.assert_called_once_with()


class TestActivationAndStateContract(unittest.TestCase):
    """The trigger directory enables SlugAudit and owns its local manifest."""

    def test_find_project_root_walks_to_nearest_trigger(self) -> None:
        with TemporaryDirectory() as tmp:
            outer = Path(tmp) / "outer"
            inner = outer / "packages" / "inner"
            nested = inner / "src" / "feature"
            nested.mkdir(parents=True)
            _activate(outer)
            _activate(inner)

            self.assertEqual(find_project_root(str(nested)), inner.resolve())

    def test_state_round_trip_preserves_freshness_identity_and_manifest(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            _activate(project)
            original = _state(project)

            save_state(str(project), original)
            loaded = load_state(str(project))

            self.assertIsNotNone(loaded)
            if loaded is None:
                self.fail("saved state did not load")
            self.assertEqual(loaded.contract_version, 1)
            self.assertEqual(loaded.schema_version, SCHEMA_VERSION)
            self.assertEqual(loaded.project_id, "project-1")
            self.assertEqual(loaded.revision_id, "revision-1")
            self.assertEqual(loaded.manifest_hash, "manifest-1")
            self.assertEqual(loaded.parser_version, PARSER_VERSION)
            self.assertEqual(loaded.languages, ["python"])
            self.assertEqual(loaded.files, original.files)

            raw = json.loads(
                (project / ".planning" / "slugaudit" / "state.json").read_text()
            )
            self.assertEqual(raw["contract_version"], 1)
            self.assertEqual(raw["revision_id"], "revision-1")
            self.assertEqual(raw["files"]["src/main.py"]["hash"], "file-hash-1")

    def test_missing_trigger_means_project_is_disabled_even_if_state_file_exists(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            state_file = project / ".planning" / "slugaudit" / "state.json"
            state_file.parent.mkdir(parents=True)
            state_file.write_text(json.dumps(_state(project).to_dict()))
            state_file.parent.rename(project / ".planning" / "disabled")

            self.assertIsNone(load_state(str(project)))

    def test_corrupt_state_is_rebuild_signal(self) -> None:
        with TemporaryDirectory() as tmp:
            project = Path(tmp)
            trigger = _activate(project)
            (trigger / "state.json").write_text("{definitely not json")

            self.assertIsNone(load_state(str(project)))


if __name__ == "__main__":
    unittest.main()
