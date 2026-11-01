-- SlugAudit session findings. One database file per project, beside
-- project.db, deleted when a new SlugAudit process first touches the
-- project. Findings are AI-authored only. A finding is bound to the content
-- hash of its file; whether it is still current is decided when it is read,
-- by comparing `source_hash` with the file's hash in project.db.
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
    evidence_revision TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_findings_path_hash ON findings (path, source_hash);
