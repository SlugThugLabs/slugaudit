"""PostgreSQL connection and upsert helpers for audit database."""

import os
import re
import json
from datetime import datetime, timezone
from typing import Optional

import psycopg2


def parse_connection_string(s: str) -> dict:
    """Parse postgresql://user:pass@host:port/dbname into a dict."""
    m = re.match(
        r'postgresql://(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?/(.+)',
        s,
    )
    if not m:
        raise ValueError(f"Invalid connection string: {s}")
    user, password, host, port, dbname = m.groups()
    return {
        "host": host or "localhost",
        "port": int(port) if port else 5432,
        "dbname": dbname,
        "user": user or "postgres",
        "password": password or "",
    }


def get_connection(connection_string: Optional[str] = None):
    """Get a PostgreSQL connection from connection string or environment variables.

    Connection string takes priority. If not provided, uses these env vars:
      PGHOST, PGPORT (default 5432), PGDATABASE, PGUSER, PGPASSWORD
    """
    if connection_string:
        params = parse_connection_string(connection_string)
    else:
        missing = []
        for name in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD"):
            if not os.environ.get(name):
                missing.append(name)
        if missing:
            names = ", ".join(missing)
            raise ValueError(
                f"Missing required environment variable(s): {names}\n"
                "  Set them in your shell or use --connection\n"
                "  Example: export PGHOST=localhost PGUSER=myuser PGPASSWORD=..."
            )
        params = {
            "host": os.environ["PGHOST"],
            "port": int(os.environ.get("PGPORT", "5432")),
            "dbname": os.environ["PGDATABASE"],
            "user": os.environ["PGUSER"],
            "password": os.environ["PGPASSWORD"],
        }
    return psycopg2.connect(**params)


class ConnectionPool:
    """Simple connection pool wrapper around psycopg2.pool.SimpleConnectionPool.

    Provides a persistent pool of connections for long-running processes
    like the MCP server. Connections are automatically returned to the
    pool when closed.

    Usage:
        pool = ConnectionPool(minconn=1, maxconn=5)
        conn = pool.getconn()
        # use conn...
        conn.close()  # returns to pool
        pool.closeall()  # at shutdown
    """

    def __init__(self, minconn: int = 1, maxconn: int = 5, connection_string: Optional[str] = None):
        """Create a connection pool.

        Args:
            minconn: Minimum number of connections to keep in the pool.
            maxconn: Maximum number of connections in the pool.
            connection_string: PostgreSQL connection string. If None, uses env vars.
        """
        self.connection_string = connection_string
        self._pool = None
        self._minconn = minconn
        self._maxconn = maxconn

    @property
    def pool(self):
        if self._pool is None:
            params = parse_connection_string(self.connection_string) if self.connection_string else None
            if params:
                self._pool = psycopg2.pool.SimpleConnectionPool(
                    self._minconn, self._maxconn, **params
                )
            else:
                # Use env vars
                self._pool = psycopg2.pool.SimpleConnectionPool(
                    self._minconn, self._maxconn,
                    host=os.environ.get("PGHOST"),
                    port=int(os.environ.get("PGPORT", "5432")),
                    dbname=os.environ.get("PGDATABASE"),
                    user=os.environ.get("PGUSER"),
                    password=os.environ.get("PGPASSWORD"),
                )
        return self._pool

    def getconn(self):
        """Get a connection from the pool."""
        return self.pool.getconn()

    def putconn(self, conn):
        """Return a connection to the pool."""
        self.pool.putconn(conn)

    def closeall(self):
        """Close all connections and destroy the pool."""
        if self._pool is not None:
            self._pool.closeall()
            self._pool = None


def get_or_create_project(conn, name: str, language: str, repo_path: str) -> str:
    """Get existing project or create a new one. Returns project UUID."""
    cur = conn.cursor()
    cur.execute("SELECT id FROM projects WHERE repo_path = %s", (repo_path,))
    row = cur.fetchone()
    if row:
        pid = row[0]
        cur.execute(
            "UPDATE projects SET primary_language = %s, updated_at = NOW() WHERE id = %s",
            (language, pid),
        )
        cur.close()
        return pid

    cur.execute(
        "INSERT INTO projects (name, primary_language, repo_path) VALUES (%s, %s, %s) RETURNING id",
        (name, language, repo_path),
    )
    pid = cur.fetchone()[0]
    conn.commit()
    cur.close()
    return pid


def upsert_file(
    conn, project_id: str, relpath: str, file_hash: str, file_size: int,
    mtime, signatures: list, force: bool = False,
) -> tuple:
    """Upsert a file record. Returns (file_id, was_updated)."""
    cur = conn.cursor()

    cur.execute(
        "SELECT id, hash FROM files WHERE project_id = %s AND path = %s",
        (project_id, relpath),
    )
    row = cur.fetchone()

    if row and not force:
        fid, existing_hash = row
        if existing_hash == file_hash:
            cur.close()
            return fid, False

    sig_json = json.dumps(signatures) if signatures else None

    if row:
        fid = row[0]
        cur.execute(
            """UPDATE files SET
               hash = %s, size = %s, last_modified_at = %s,
               signature_cache = %s, updated_at = NOW()
               WHERE id = %s""",
            (file_hash, file_size, mtime, sig_json, fid),
        )
    else:
        cur.execute(
            """INSERT INTO files
               (project_id, path, hash, size, last_modified_at, signature_cache)
               VALUES (%s, %s, %s, %s, %s, %s)
               RETURNING id""",
            (project_id, relpath, file_hash, file_size, mtime, sig_json),
        )
        fid = cur.fetchone()[0]

    conn.commit()
    cur.close()
    return fid, True


def delete_removed_files(conn, project_id: str, active_paths: set):
    """Remove files from DB that no longer exist on disk."""
    cur = conn.cursor()
    cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
    for fid, path in cur.fetchall():
        if path not in active_paths:
            for tbl in ("dependency_edges", "file_imports", "file_staleness"):
                cur.execute(f"DELETE FROM {tbl} WHERE file_id = %s OR source_file_id = %s", (fid, fid))
            cur.execute("DELETE FROM findings WHERE file_id = %s", (fid,))
            cur.execute("DELETE FROM files WHERE id = %s", (fid,))
    conn.commit()
    cur.close()


def insert_imports(conn, project_id: str, file_id: str, imports: list[dict], force: bool = False):
    """Insert import records for a file. Clears old ones if force."""
    cur = conn.cursor()
    if force:
        cur.execute("DELETE FROM file_imports WHERE file_id = %s", (file_id,))
    for imp in imports:
        cur.execute(
            """INSERT INTO file_imports
               (project_id, file_id, import_text, resolved_path, import_type, line_start, line_end)
               VALUES (%s, %s, %s, %s, %s, %s, %s)""",
            (
                project_id,
                file_id,
                imp["import_text"],
                imp.get("resolved_path"),
                imp.get("import_type", "internal"),
                imp.get("line_start"),
                imp.get("line_end"),
            ),
        )
    conn.commit()
    cur.close()


def build_dependency_edges(conn, project_id: str, importer, force: bool = False):
    """Resolve all file_imports to dependency edges between files."""
    cur = conn.cursor()
    if force:
        cur.execute("DELETE FROM dependency_edges WHERE project_id = %s", (project_id,))

    cur.execute("SELECT id, path FROM files WHERE project_id = %s", (project_id,))
    file_map = dict(cur.fetchall())
    path_to_id = {v: k for k, v in file_map.items()}

    cur.execute(
        """SELECT fi.id, fi.file_id, fi.import_text, f.path
           FROM file_imports fi
           JOIN files f ON f.id = fi.file_id
           WHERE fi.project_id = %s AND fi.import_type = 'internal'
           AND fi.resolved_path IS NULL""",
        (project_id,),
    )
    rows = cur.fetchall()

    edges_added = 0
    for import_id, src_file_id, import_text, src_path in rows:
        resolved = importer.resolve_import(import_text, src_path, path_to_id)
        if resolved and resolved in path_to_id:
            target_id = path_to_id[resolved]
            if target_id != src_file_id:
                try:
                    cur.execute(
                        """INSERT INTO dependency_edges
                           (project_id, source_file_id, target_file_id, import_id)
                           VALUES (%s, %s, %s, %s)
                           ON CONFLICT DO NOTHING""",
                        (project_id, src_file_id, target_id, import_id),
                    )
                    if cur.rowcount > 0:
                        edges_added += 1
                except Exception:
                    pass
            cur.execute(
                "UPDATE file_imports SET resolved_path = %s WHERE id = %s",
                (resolved, import_id),
            )

    conn.commit()
    cur.close()
    return edges_added


def update_audit_timestamps(conn, project_id: str):
    """Update last_audited_hash to match current hash for all files."""
    cur = conn.cursor()
    cur.execute(
        "UPDATE files SET last_audited_hash = hash WHERE project_id = %s",
        (project_id,),
    )
    conn.commit()
    cur.close()


def get_changed_files(conn, project_id: str) -> list[tuple]:
    """Return (id, path) for files where hash differs from last_audited_hash."""
    cur = conn.cursor()
    cur.execute(
        """SELECT id, path FROM files
           WHERE project_id = %s
           AND (last_audited_hash IS NULL OR hash != last_audited_hash)
           ORDER BY path""",
        (project_id,),
    )
    rows = cur.fetchall()
    cur.close()
    return rows


def get_project_names(conn) -> list[str]:
    """Return all project names."""
    cur = conn.cursor()
    cur.execute("SELECT name FROM projects ORDER BY name")
    rows = [r[0] for r in cur.fetchall()]
    cur.close()
    return rows


def schema_exists(conn) -> bool:
    """Check if the audit-db schema has been initialized.

    Returns True if the 'projects' table exists, False otherwise.
    This allows import commands to auto-initialize the schema on first use.
    """
    try:
        cur = conn.cursor()
        cur.execute(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables "
            "WHERE table_name = 'projects')"
        )
        result = cur.fetchone()[0]
        cur.close()
        return result
    except Exception:
        return False


def update_audit_timestamps(conn, project_id: str):
    """Update last_audited_hash to match current hash for all files.

    Called after a successful import to mark all files as audited.
    This is used for incremental imports — files that haven't changed
    won't appear as targets in the next briefing.
    """
    cur = conn.cursor()
    cur.execute(
        "UPDATE files SET last_audited_hash = hash WHERE project_id = %s",
        (project_id,),
    )
    conn.commit()
    cur.close()
