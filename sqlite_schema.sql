-- ============================================================================
-- SQLite Audit Database Schema
-- Zero-config local backend, used only when no PostgreSQL is configured.
-- Scoped to exactly the 7 tables the current tool surface reads or writes —
-- see services/sqlite_schema_service.py and CLAUDE.md for why this is
-- deliberately smaller than schema.sql (PostgreSQL's schema carries several
-- tables from a deleted feature set that nothing here uses).
--
-- UUIDs are generated in Python (no gen_random_uuid() equivalent) and stored
-- as TEXT. JSON is stored as TEXT (json.dumps/loads at the repository
-- boundary). Timestamps are ISO 8601 UTC TEXT. Foreign keys require
-- `PRAGMA foreign_keys = ON` per connection (infrastructure/sqlite_db.py
-- sets this on every connection it opens).
-- ============================================================================

CREATE TABLE IF NOT EXISTS schema_migrations (
    version         INTEGER PRIMARY KEY,
    description     TEXT NOT NULL,
    applied_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS projects (
    id                   TEXT PRIMARY KEY,
    name                 TEXT NOT NULL,
    primary_language     TEXT,
    repo_path            TEXT NOT NULL,
    current_revision_id  TEXT REFERENCES project_revisions(id) ON DELETE SET NULL,
    created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at           TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS uniq_projects_repo_path ON projects(repo_path);
CREATE INDEX IF NOT EXISTS idx_projects_name ON projects(name);

-- A revision is visible to readers only after its status becomes ready and
-- projects.current_revision_id points at it in the same transaction.
CREATE TABLE IF NOT EXISTS project_revisions (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    manifest_hash   TEXT NOT NULL,
    file_count      INTEGER NOT NULL DEFAULT 0,
    signature_count INTEGER NOT NULL DEFAULT 0,
    parser_version  TEXT,
    status          TEXT NOT NULL DEFAULT 'building'
                    CHECK (status IN ('building', 'ready', 'failed')),
    error_message   TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    published_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_project_revisions_project
    ON project_revisions(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_project_revisions_manifest
    ON project_revisions(project_id, manifest_hash);

CREATE TABLE IF NOT EXISTS files (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path              TEXT NOT NULL,
    content           TEXT,
    hash              TEXT,
    size              INTEGER,
    last_audited_at   TEXT,
    last_modified_at  TEXT,
    signature_cache   TEXT,  -- JSON array, decoded at the repository boundary
    last_audited_hash TEXT,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at        TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS uniq_files_project_path ON files(project_id, path);
CREATE INDEX IF NOT EXISTS idx_files_hash ON files(project_id, hash);
CREATE INDEX IF NOT EXISTS idx_files_last_audited ON files(project_id, last_audited_hash);

CREATE TABLE IF NOT EXISTS file_imports (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    file_id       TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    import_text   TEXT NOT NULL,
    resolved_path TEXT,
    import_type   TEXT DEFAULT 'internal',
    line_start    INTEGER,
    line_end      INTEGER,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_file_imports_file ON file_imports(file_id);
CREATE INDEX IF NOT EXISTS idx_file_imports_project ON file_imports(project_id);
CREATE INDEX IF NOT EXISTS idx_file_imports_unresolved
    ON file_imports(project_id, import_type) WHERE resolved_path IS NULL;

CREATE TABLE IF NOT EXISTS dependency_edges (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_file_id  TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    target_file_id  TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    import_id       TEXT REFERENCES file_imports(id) ON DELETE SET NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (source_file_id, target_file_id, import_id)
);

CREATE INDEX IF NOT EXISTS idx_dep_edges_source ON dependency_edges(source_file_id);
CREATE INDEX IF NOT EXISTS idx_dep_edges_target ON dependency_edges(target_file_id);
CREATE INDEX IF NOT EXISTS idx_dep_edges_project ON dependency_edges(project_id);

CREATE TABLE IF NOT EXISTS risk_patterns (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    file_id         TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pattern_type    TEXT NOT NULL,
    count           INTEGER NOT NULL DEFAULT 1,
    line_start      INTEGER,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (file_id, pattern_type, line_start)
);

CREATE INDEX IF NOT EXISTS idx_risk_patterns_file ON risk_patterns(file_id);
CREATE INDEX IF NOT EXISTS idx_risk_patterns_project ON risk_patterns(project_id);

CREATE TABLE IF NOT EXISTS findings (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    file_id         TEXT REFERENCES files(id) ON DELETE SET NULL,
    identity_hash   TEXT,
    severity        TEXT,
    blast_radius    TEXT,
    proximity       TEXT,
    risk_score      REAL,
    category        TEXT,
    message         TEXT,
    line_start      INTEGER,
    line_end        INTEGER,
    status          TEXT NOT NULL DEFAULT 'open',
    triage_source   TEXT DEFAULT 'auto',
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT
);

CREATE INDEX IF NOT EXISTS idx_findings_project ON findings(project_id, status);
CREATE INDEX IF NOT EXISTS idx_findings_file ON findings(file_id);
CREATE INDEX IF NOT EXISTS idx_findings_severity ON findings(severity);

INSERT OR IGNORE INTO schema_migrations (version, description)
VALUES (1, 'initial SQLite schema: 7-table scope matching current tool surface');
