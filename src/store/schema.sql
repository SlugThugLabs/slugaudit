-- SlugAudit per-project schema. One database file per project; there is
-- exactly one project row and at most one current revision at a time.
-- No risk_pattern table, and no table whose purpose is to claim SlugAudit
-- found a bug: findings are AI-authored only.

CREATE TABLE IF NOT EXISTS project (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    project_id TEXT NOT NULL,
    root_path TEXT NOT NULL,
    contract_version INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    created_at_unix INTEGER NOT NULL
);

-- Append-only log of published revisions. Atomicity/isolation of a publish
-- comes from SQLite's own WAL transaction semantics (a reader never sees a
-- partial commit) — this table is freshness metadata for the AI, not a
-- mechanism the store depends on for correctness. files/evidence/
-- dependency_edges hold only current state and are never duplicated per
-- revision; a changed file is replaced in place, a deleted file is removed.
CREATE TABLE IF NOT EXISTS revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    revision_id TEXT NOT NULL UNIQUE,
    manifest_hash TEXT NOT NULL,
    parser_pack_version TEXT NOT NULL,
    created_at_unix INTEGER NOT NULL,
    is_current INTEGER NOT NULL DEFAULT 0 CHECK (is_current IN (0, 1))
);

-- At most one current revision at any time; readers bind to it and never
-- observe a half-published one.
CREATE UNIQUE INDEX IF NOT EXISTS idx_revisions_single_current
    ON revisions (is_current)
    WHERE is_current = 1;

CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    file_kind TEXT NOT NULL DEFAULT 'indexed' CHECK (file_kind IN ('indexed', 'binary')),
    content TEXT,
    content_hash TEXT NOT NULL,
    hash_algorithm TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    modified_unix_seconds INTEGER,
    language TEXT,
    language_detected INTEGER NOT NULL DEFAULT 0 CHECK (language_detected IN (0, 1)),
    -- These three CHECK lists are the SQL mirror of
    -- model::parser::{ParserAvailability, ParseOutcome, ExtractionCompleteness}
    -- ::as_sql_text() — keep both in sync if a variant is ever added.
    parser_availability TEXT NOT NULL
        CHECK (parser_availability IN ('Available', 'Unavailable', 'LoadFailed')),
    parse_outcome TEXT NOT NULL
        CHECK (parse_outcome IN ('NotAttempted', 'Succeeded', 'SyntaxErrors', 'Failed')),
    -- Set only when parser_availability = 'LoadFailed' or parse_outcome =
    -- 'Failed'. Per-syntax-error detail lives in the evidence table as
    -- Diagnostic rows; this column is for the harder failure where process()
    -- itself returned an error rather than a tree with error nodes.
    parse_error_reason TEXT,
    extraction_completeness TEXT NOT NULL
        CHECK (extraction_completeness IN ('Full', 'Partial', 'ContentOnly', 'Unavailable')),
    last_revision_id INTEGER NOT NULL REFERENCES revisions (id)
);

CREATE INDEX IF NOT EXISTS idx_files_hash ON files (content_hash);

-- One generic evidence table backs structures, imports, exports, comments,
-- docstrings, symbols, diagnostics, chunks, and raw-tree fallback nodes —
-- the kinds in src/model/evidence.rs::EvidenceKind. Fields common to every
-- kind (span, origin, provenance) are real columns so `query` can filter
-- and index on them; kind-specific fields live in `payload` as JSON.
-- Scoped only to its file — deleting a file cascades away its evidence.
CREATE TABLE IF NOT EXISTS evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    kind TEXT NOT NULL,
    origin TEXT NOT NULL,
    span_availability TEXT NOT NULL,
    start_byte INTEGER,
    end_byte INTEGER,
    start_line INTEGER,
    start_column INTEGER,
    end_line INTEGER,
    end_column INTEGER,
    payload TEXT NOT NULL,
    UNIQUE (file_id, key)
);

CREATE INDEX IF NOT EXISTS idx_evidence_kind ON evidence (kind);
CREATE INDEX IF NOT EXISTS idx_evidence_file ON evidence (file_id);

-- Derived from import evidence during sync (src/graph/). Kept separate from
-- `evidence` because traversal needs real from/to columns for recursive
-- CTEs, not JSON-encoded ones. Cascades away with either endpoint file.
CREATE TABLE IF NOT EXISTS dependency_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    to_file_id INTEGER REFERENCES files (id) ON DELETE CASCADE,
    raw_import_text TEXT NOT NULL,
    -- Mirrors src/graph/resolve.rs::ResolutionKind::as_sql_text() — keep
    -- both in sync if a variant is ever added.
    resolution_kind TEXT NOT NULL CHECK (resolution_kind IN ('Resolved', 'Unresolved', 'External')),
    confidence TEXT CHECK (confidence IS NULL OR confidence IN ('High', 'Low'))
);

CREATE INDEX IF NOT EXISTS idx_edges_from ON dependency_edges (from_file_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON dependency_edges (to_file_id);

-- Findings are AI-authored only and outlive any single revision: they are
-- invalidated by comparing source_hash against the current file, not by a
-- foreign key to a revision that may since have been replaced.
CREATE TABLE IF NOT EXISTS findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    line_start INTEGER NOT NULL CHECK (line_start > 0),
    line_end INTEGER NOT NULL CHECK (line_end >= line_start),
    severity TEXT NOT NULL CHECK (length(severity) BETWEEN 1 AND 100),
    category TEXT NOT NULL CHECK (length(category) BETWEEN 1 AND 100),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 10000),
    created_at_unix INTEGER NOT NULL,
    evidence_revision TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'current' CHECK (status IN ('current', 'stale'))
);

CREATE INDEX IF NOT EXISTS idx_findings_path_hash ON findings (path, source_hash);
