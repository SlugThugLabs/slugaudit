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

-- One row per file the project contains — not per file that was indexed. A
-- discovered file that could not be read (oversize, unreadable) gets an
-- existence row with file_kind = 'skipped' and no content, so it is
-- countable instead of invisible in every `count(*) FROM files`.
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    -- 'indexed' (text sampled and parsed), 'binary' (sampled, inert), or
    -- 'skipped' (discovered but deliberately not read). 'skipped' rows are
    -- written by `sync::revision_skipped::upsert_skipped_file`.
    file_kind TEXT NOT NULL DEFAULT 'indexed'
        CHECK (file_kind IN ('indexed', 'binary', 'skipped')),
    -- Why a 'skipped' row was not indexed, as one of a few short stable codes
    -- (`sync::revision_skipped::SKIP_REASON_*`) rather than the per-file
    -- human message — which embeds each file's own path and size and so
    -- could never group. 'oversize' = over the per-file import ceiling;
    -- 'unreadable' = the read failed; 'stat_failed' = the stat failed so the
    -- size is unknown (byte_len NULL). NULL for every other kind: those rows
    -- were read, so nothing was skipped.
    skip_reason TEXT,
    -- Where the file came from, as a fixed set rather than a judgment:
    -- 'authored' (neither below — the default, so a row written by older
    -- code or a test fixture still carries the truthful value), 'generated'
    -- (a machine-written file: a `generated` path segment or filename
    -- token, or a codegen suffix such as `.pb.go` / `.g.dart` / `.min.js`),
    -- or 'vendored' (a third-party tree: `vendor/`, `node_modules/`,
    -- `third_party/`, `site-packages/`, …). Derived from the path alone by
    -- `sync::classify::classify` — never from file content — so it costs
    -- nothing to store and is available for skipped rows too. It is a path
    -- heuristic: a hand-written file named `*generated*` is classified
    -- `generated`. 'vendored' wins when a path matches both (a generated
    -- file inside `vendor/` is vendored — the third-party fact already
    -- explains it). No score, severity, or ranking: a count of vendored
    -- rows is a fact about where code came from, not about whether it
    -- matters.
    file_class TEXT NOT NULL DEFAULT 'authored'
        CHECK (file_class IN ('authored', 'generated', 'vendored')),
    content TEXT,
    -- BLAKE3 over the file's content for 'indexed'/'binary' rows
    -- (hash_algorithm = 'blake3-v1'). A 'skipped' row retains no content,
    -- and reading the file to hash it would defeat the skip, so its
    -- identity is stat-derived instead: BLAKE3 over (path, byte_len,
    -- modified_unix_seconds) with hash_algorithm = 'stat-identity'
    -- (`sync::hash::stat_identity`). Consequence, accepted and identical to
    -- the stat sweep's documented blind spot: a skipped file whose content
    -- changes without changing its size or mtime is not detected.
    content_hash TEXT NOT NULL,
    hash_algorithm TEXT NOT NULL,
    -- NULL only for a 'skipped' row whose stat failed (skip_reason =
    -- 'stat_failed'): the size is unknown, which is not the same as a real
    -- empty file's 0. Every other row carries the real size. A SUM over
    -- this column ignores unknowns, so pair it with count(*) WHERE
    -- byte_len IS NULL.
    byte_len INTEGER,
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
-- docstrings, symbols, diagnostics, and call sites — the kinds in
-- src/model/evidence.rs::EvidenceKind. Every variant here has a producer in
-- `src/evidence/` or `src/sync/`: `src/bin/check_producer_coverage` fails
-- the gate when one does not. Fields common to every
-- kind (span, origin, provenance) are real columns so `query` can filter
-- and index on them; kind-specific fields live in `payload` as JSON.
-- Scoped only to its file — deleting a file cascades away its evidence.
CREATE TABLE IF NOT EXISTS evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    -- The SQL mirror of src/evidence/sql.rs::kind_text(), which maps every
    -- model::EvidenceKind variant (src/model/evidence.rs). A variant outside
    -- this list is a bug in the writer, not data, so the mirror has to stay
    -- exact in both directions: `store::connection::tests::
    -- every_evidence_kind_text_passes_the_kind_check` derives every variant
    -- from the enum itself and fails until this list carries its text, and
    -- `src/bin/check_producer_coverage` fails until something under `src/`
    -- actually constructs it — no kind may be advertised that cannot occur.
    kind TEXT NOT NULL
        CHECK (kind IN ('Structure', 'Import', 'Export', 'Comment', 'Docstring',
                        'Symbol', 'Diagnostic', 'Call')),
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
    confidence TEXT CHECK (confidence IS NULL OR confidence IN ('High', 'Low')),
    syntax_unmodeled INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_edges_from ON dependency_edges (from_file_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON dependency_edges (to_file_id);

-- Findings are AI-authored only.
-- The session_id column supports the session-scoped cleanup
-- (`sync::manager_meta::purge_prior_session_findings`) on fresh-process boots.
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
    status TEXT NOT NULL DEFAULT 'current' CHECK (status IN ('current', 'stale')),
    session_id TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_findings_path_hash ON findings (path, source_hash);
CREATE INDEX IF NOT EXISTS idx_findings_session ON findings (session_id);
