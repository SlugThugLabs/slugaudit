"""Atomic, polyglot filesystem-to-database reconciliation."""

from __future__ import annotations

import hashlib
import os
from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from app.manifest import PARSER_VERSION, SourceManifest, build_manifest
from domain import ImportResult
from infrastructure import IFileSystem, get_connection, get_file_system
from languages import LANG_MAP
from repositories import (
    FileRepository,
    ImportRepository,
    ProjectRepository,
    RiskPatternRepository,
    repository_transaction,
)


def _build_sig_cache(sigs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Keep the AST evidence an AI needs without storing full syntax trees."""
    fields = (
        "type",
        "name",
        "visibility",
        "signature",
        "generic_params",
        "doc_comment",
        "line_start",
        "line_end",
        "is_async",
        "is_unsafe",
    )
    return [
        {field: sig[field] for field in fields if field in sig and sig[field] not in (None, "")}
        for sig in sigs
    ]


def _build_import_records(imps: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "import_text": imp["import_text"],
            "resolved_path": None,
            "import_type": imp.get("import_type", "internal"),
            "line_start": imp.get("line_start"),
            "line_end": imp.get("line_end"),
        }
        for imp in imps
    ]


class _PolyglotResolver:
    """Dispatch import resolution using the source file's language."""

    def __init__(self, extractors: dict[str, Any]):
        self.extractors = extractors

    def resolve_import(
        self,
        import_text: str,
        source_file: str,
        path_to_id: dict[str, Any],
    ) -> str | None:
        suffix = Path(source_file).suffix.lower()
        for extractor in self.extractors.values():
            if suffix in extractor.source_extensions():
                resolved = extractor.resolve_import(import_text, source_file, path_to_id)
                return str(resolved) if resolved is not None else None
        return None


class ImportService:
    """Reconcile a complete manifest into the current project index."""

    def __init__(
        self,
        extractor: Any | None = None,
        file_system: IFileSystem | None = None,
    ):
        # ``extractor`` remains accepted for callers of the former single-language API.
        self.extractor = extractor
        self.file_system = file_system or get_file_system()

    def _extractors(self, project_root: str, manifest: SourceManifest) -> dict[str, Any]:
        extractors: dict[str, Any] = {}
        for language in manifest.languages:
            extractor_type = LANG_MAP.get(language)
            if extractor_type is None:
                raise ValueError(f"No Tree-sitter extractor registered for {language}")
            extractors[language] = extractor_type(project_root)
        return extractors

    def reconcile_project(
        self,
        project_path: str,
        manifest: SourceManifest,
        *,
        conn: Any,
        force_full: bool = False,
        on_progress: Callable[..., Any] | None = None,
    ) -> ImportResult:
        """Publish one all-or-nothing current revision.

        The caller supplies the disk manifest built immediately before this
        operation. Every changed file is hashed again while read; a concurrent
        filesystem mutation aborts the transaction instead of publishing mixed
        evidence.
        """
        project_root = str(Path(project_path).resolve())
        if not os.path.isdir(project_root):
            raise ValueError(f"Not a directory: {project_path}")

        started = datetime.now(UTC)
        extractors = self._extractors(project_root, manifest)
        resolver = _PolyglotResolver(extractors)

        project_repo = ProjectRepository(conn, auto_commit=False)
        file_repo = FileRepository(conn, auto_commit=False)
        import_repo = ImportRepository(conn, auto_commit=False)
        risk_repo = RiskPatternRepository(conn, auto_commit=False)

        result = ImportResult()
        with repository_transaction(conn):
            language_summary = (
                manifest.languages[0]
                if len(manifest.languages) == 1
                else "polyglot" if manifest.languages else "unknown"
            )
            project_id = project_repo.get_or_create(
                name=Path(project_root).name,
                repo_path=project_root,
                language=language_summary,
            )

            db_manifest = file_repo.get_manifest(project_id)
            disk_hashes = {
                path: source.hash for path, source in manifest.files.items()
            }
            if force_full:
                file_repo.delete_removed(project_id, set())
                db_manifest = {}

            added_paths = sorted(set(disk_hashes).difference(db_manifest))
            modified_paths = sorted(
                path
                for path in set(disk_hashes).intersection(db_manifest)
                if disk_hashes[path] != db_manifest[path]
            )
            changed_paths = added_paths + modified_paths

            for index, relpath in enumerate(changed_paths, 1):
                source = manifest.files[relpath]
                absolute_path = os.path.join(project_root, relpath)
                content = self.file_system.read_file_bytes(absolute_path)
                observed_hash = hashlib.sha256(content).hexdigest()
                if observed_hash != source.hash:
                    raise RuntimeError(
                        f"Source changed while SlugAudit was syncing: {relpath}"
                    )

                extractor = extractors[source.language]
                signatures = extractor.extract_signatures(absolute_path, content)
                imports = extractor.extract_imports(absolute_path, content)
                risks = extractor.extract_risk_patterns(absolute_path, content)
                modified_at = datetime.fromtimestamp(
                    self.file_system.get_mtime(absolute_path), tz=UTC
                ).isoformat()

                file_id, _ = file_repo.upsert(
                    project_id=project_id,
                    relpath=relpath,
                    file_hash=source.hash,
                    file_size=source.size,
                    mtime=modified_at,
                    signatures=_build_sig_cache(signatures),
                    content=content.decode("utf-8", errors="replace"),
                    force=True,
                )
                if relpath in modified_paths:
                    file_repo.purge_obsolete_findings(project_id, file_id)
                import_repo.insert(
                    project_id,
                    file_id,
                    _build_import_records(imports),
                    force=True,
                )
                risk_repo.upsert(project_id, file_id, risks)
                if on_progress:
                    on_progress(index, len(changed_paths))

            removed_count = file_repo.delete_removed(project_id, set(disk_hashes))
            edge_count = import_repo.build_dependency_edges(
                project_id, resolver, force=True
            )
            file_repo.update_audit_timestamps(project_id)
            stats = project_repo.get_status(project_id)

            revision_id = project_repo.begin_revision(
                project_id=project_id,
                manifest_hash=manifest.manifest_hash,
                file_count=manifest.file_count,
                signature_count=stats["signatures_count"],
                parser_version=PARSER_VERSION,
            )
            project_repo.publish_revision(project_id, revision_id)

            result.project_id = project_id
            result.project_name = Path(project_root).name
            result.language = language_summary
            result.files_processed = stats["file_count"]
            result.signatures_extracted = stats["signatures_count"]
            result.imports_extracted = stats["imports_count"]
            result.dependency_edges = edge_count
            result.risk_patterns = sum(risk_repo.get_pattern_summary(project_id).values())
            result.revision_id = revision_id
            result.manifest_hash = manifest.manifest_hash
            result.added_files = len(added_paths)
            result.modified_files = len(modified_paths)
            result.deleted_files = removed_count

        result.elapsed_seconds = (
            datetime.now(UTC) - started
        ).total_seconds()
        return result

    def import_project(
        self,
        project_path: str,
        project_name: str | None = None,
        language: str | None = "auto",
        connection_string: str | None = None,
        on_progress: Callable[..., Any] | None = None,
        conn: Any | None = None,
    ) -> ImportResult:
        """Compatibility wrapper that performs a complete atomic reconciliation."""
        del project_name, language
        owned_connection = conn is None
        if conn is None:
            conn = get_connection(connection_string)
        try:
            return self.reconcile_project(
                project_path,
                build_manifest(project_path),
                conn=conn,
                force_full=True,
                on_progress=on_progress,
            )
        finally:
            if owned_connection:
                conn.close()


def reconcile_project(
    project_path: str,
    manifest: SourceManifest,
    *,
    conn: Any,
    force_full: bool = False,
) -> ImportResult:
    """Canonical import entry point used by automatic MCP synchronization."""
    return ImportService().reconcile_project(
        project_path,
        manifest,
        conn=conn,
        force_full=force_full,
    )


def import_project(
    project_path: str,
    project_name: str | None = None,
    language: str | None = "auto",
    connection_string: str | None = None,
    on_progress: Callable[..., Any] | None = None,
    conn: Any | None = None,
) -> ImportResult:
    """Backward-compatible full import entry point."""
    return ImportService().import_project(
        project_path=project_path,
        project_name=project_name,
        language=language,
        connection_string=connection_string,
        on_progress=on_progress,
        conn=conn,
    )


__all__ = ["ImportService", "import_project", "reconcile_project"]
