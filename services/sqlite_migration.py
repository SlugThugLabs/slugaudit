"""One-time migration: carry findings forward when PostgreSQL becomes
available for a project that was previously running on the zero-config
SQLite backend.

Only findings are migrated. Everything else the SQLite backend stores
(files, signatures, imports, dependency edges, risk patterns) is re-derived
from the actual source files on every sync regardless of backend — that's
the whole "mandatory freshness, no stale fallback" design (app/sync.py) — so
a fresh PostgreSQL sync regenerates all of that for free. Findings are the
one thing that represents actual AI judgment rather than something
recomputed from source, so they're the one thing worth carrying forward.

A migrated finding keeps its original identity_hash rather than recomputing
one: identity_hash is only ever used as a dedup key (FindingRepository.
record()), so reusing it makes re-running this migration safely idempotent,
and any future natural audit_finding call against the same location
computes its own fresh identity_hash from the current file content anyway,
so it can never collide with or get confused for a migrated one.

A finding whose file no longer exists at its recorded path is correctly not
carried forward, matching the existing "findings are purged when their
source evidence changes" invariant. A finding whose file exists but whose
*content* has changed since the finding was recorded is migrated anyway —
there is no way to detect that from what's stored (identity_hash is a
one-way hash and findings don't separately store the file_hash they were
recorded against) — but the very next sync that actually changes that file
purges it via the same purge_obsolete_findings() path any organically
created finding goes through, so this self-heals within one sync cycle.
"""

import logging
import sqlite3
from pathlib import Path
from typing import Any

from infrastructure.sqlite_db import sqlite_db_path
from repositories import make_file_repository, make_finding_repository

logger = logging.getLogger("slugaudit-mcp.sqlite_migration")


def migrate_sqlite_findings(project_root: str | Path, project_id: str, pg_conn: Any) -> int:
    """Migrate open findings from a leftover local SQLite db into PostgreSQL.

    Returns the number of findings migrated. Renames the old SQLite file to
    ``audit.db.migrated`` afterward (never deletes it) so this runs at most
    once per leftover file — safe to call on every sync in the meantime,
    since the os.path.exists() check below is cheap and short-circuits to a
    no-op the moment nothing is left to migrate.
    """
    old_db_path = sqlite_db_path(project_root)
    if not old_db_path.exists():
        return 0

    old_conn = sqlite3.connect(str(old_db_path))
    try:
        cur = old_conn.execute(
            """SELECT f.path, fi.identity_hash, fi.severity, fi.category,
                      fi.message, fi.line_start, fi.line_end
               FROM findings fi
               JOIN files f ON f.id = fi.file_id
               WHERE fi.status = 'open'"""
        )
        old_findings = cur.fetchall()
    finally:
        old_conn.close()

    file_repo = make_file_repository(pg_conn)
    finding_repo = make_finding_repository(pg_conn)

    migrated = 0
    for path, identity_hash, severity, category, message, line_start, line_end in old_findings:
        identity = file_repo.get_file_identity(project_id, path)
        if identity is None:
            continue
        file_id, _ = identity
        finding_repo.record(
            project_id=project_id,
            file_id=file_id,
            identity_hash=identity_hash,
            severity=severity,
            category=category,
            message=message,
            line_start=line_start,
            line_end=line_end,
        )
        migrated += 1

    old_db_path.rename(old_db_path.with_name(old_db_path.name + ".migrated"))
    return migrated


__all__ = ["migrate_sqlite_findings"]
