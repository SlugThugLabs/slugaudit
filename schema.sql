-- ============================================================================
-- Audit Database Schema
-- PostgreSQL 15+ required (uses uuid, gen_random_uuid, jsonb, timestamptz)
-- ============================================================================

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ============================================================================
-- Core Entities
-- ============================================================================

CREATE TABLE IF NOT EXISTS projects (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    primary_language TEXT,
    repo_path       TEXT NOT NULL,
    config_id       UUID,  -- FK added below (circular ref with audit_configs)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS audit_configs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id        UUID NOT NULL,
    version           INT NOT NULL DEFAULT 1,
    modules_enabled   JSONB,
    module_prompts    JSONB,
    severity_criteria JSONB,
    token_budget      INT DEFAULT 80000,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS audit_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      UUID NOT NULL,
    config_id       UUID,
    scope           TEXT,
    modules_run     JSONB,
    files_in_scope  INT,
    files_skipped   INT,
    briefing_document TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    status          TEXT NOT NULL DEFAULT 'in_progress',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- Files and Signatures
-- ============================================================================

CREATE TABLE IF NOT EXISTS files (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id        UUID NOT NULL,
    path              TEXT NOT NULL,
    hash              TEXT,
    size              BIGINT,
    last_audited_at   TIMESTAMPTZ,
    last_modified_at  TIMESTAMPTZ,
    signature_cache   JSONB,
    last_audited_hash TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS file_imports (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id    UUID NOT NULL,
    file_id       UUID NOT NULL,
    import_text   TEXT NOT NULL,
    resolved_path TEXT,
    import_type   TEXT DEFAULT 'internal',
    line_start    INT,
    line_end      INT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- Dependency Analysis
-- ============================================================================

CREATE TABLE IF NOT EXISTS dependency_edges (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      UUID NOT NULL,
    source_file_id  UUID NOT NULL,
    target_file_id  UUID NOT NULL,
    import_id       UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uniq_dependency_edge UNIQUE (source_file_id, target_file_id, import_id)
);

CREATE TABLE IF NOT EXISTS file_staleness (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    file_id         UUID NOT NULL,
    run_id          UUID,
    reason          TEXT,
    flagged_by      TEXT,
    source_file_id  UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- Findings and Audit Results
-- ============================================================================

CREATE TABLE IF NOT EXISTS findings (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id          UUID NOT NULL,
    file_id             UUID,
    run_id              UUID,
    config_id           UUID,
    module              TEXT,
    identity_hash       TEXT,
    severity            TEXT,
    blast_radius        TEXT,
    proximity           TEXT,
    risk_score          NUMERIC,
    category            TEXT,
    message             TEXT,
    line_start          INT,
    line_end            INT,
    status              TEXT NOT NULL DEFAULT 'open',
    triage_source       TEXT DEFAULT 'auto',
    resolution_reason   TEXT,
    first_seen_run_id   UUID,
    last_seen_run_id    UUID,
    suppressed_at       TIMESTAMPTZ,
    suppressed_reason   TEXT,
    suppression_expiry  TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS architecture_state (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL,
    run_id      UUID NOT NULL,
    summary     TEXT NOT NULL,
    layer_map   JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS static_tool_results (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    file_id          UUID NOT NULL,
    run_id           UUID,
    tool_name        TEXT NOT NULL,
    raw_output       TEXT,
    parsed_findings  JSONB,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS ingestor_rejections (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id             UUID NOT NULL,
    attempt_number     INT,
    missing_finding_ids JSONB,
    ai_raw_output      TEXT,
    rejected_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- Foreign Keys (deferred to avoid circular dependency issues)
-- ============================================================================

ALTER TABLE projects ADD CONSTRAINT fk_projects_config
    FOREIGN KEY (config_id) REFERENCES audit_configs(id) DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE audit_configs ADD CONSTRAINT fk_audit_configs_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;

ALTER TABLE audit_runs ADD CONSTRAINT fk_audit_runs_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE audit_runs ADD CONSTRAINT fk_audit_runs_config
    FOREIGN KEY (config_id) REFERENCES audit_configs(id);

ALTER TABLE files ADD CONSTRAINT fk_files_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;

ALTER TABLE file_imports ADD CONSTRAINT fk_file_imports_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE file_imports ADD CONSTRAINT fk_file_imports_file
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE;

ALTER TABLE dependency_edges ADD CONSTRAINT fk_dep_edges_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE dependency_edges ADD CONSTRAINT fk_dep_edges_source
    FOREIGN KEY (source_file_id) REFERENCES files(id) ON DELETE CASCADE;
ALTER TABLE dependency_edges ADD CONSTRAINT fk_dep_edges_target
    FOREIGN KEY (target_file_id) REFERENCES files(id) ON DELETE CASCADE;
ALTER TABLE dependency_edges ADD CONSTRAINT fk_dep_edges_import
    FOREIGN KEY (import_id) REFERENCES file_imports(id) ON DELETE SET NULL;

ALTER TABLE file_staleness ADD CONSTRAINT fk_file_staleness_file
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE;
ALTER TABLE file_staleness ADD CONSTRAINT fk_file_staleness_run
    FOREIGN KEY (run_id) REFERENCES audit_runs(id);
ALTER TABLE file_staleness ADD CONSTRAINT fk_file_staleness_source
    FOREIGN KEY (source_file_id) REFERENCES files(id) ON DELETE SET NULL;

ALTER TABLE findings ADD CONSTRAINT fk_findings_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE findings ADD CONSTRAINT fk_findings_file
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE SET NULL;
ALTER TABLE findings ADD CONSTRAINT fk_findings_run
    FOREIGN KEY (run_id) REFERENCES audit_runs(id);
ALTER TABLE findings ADD CONSTRAINT fk_findings_config
    FOREIGN KEY (config_id) REFERENCES audit_configs(id);
ALTER TABLE findings ADD CONSTRAINT fk_findings_first_seen_run
    FOREIGN KEY (first_seen_run_id) REFERENCES audit_runs(id);
ALTER TABLE findings ADD CONSTRAINT fk_findings_last_seen_run
    FOREIGN KEY (last_seen_run_id) REFERENCES audit_runs(id);

ALTER TABLE architecture_state ADD CONSTRAINT fk_arch_state_project
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE;
ALTER TABLE architecture_state ADD CONSTRAINT fk_arch_state_run
    FOREIGN KEY (run_id) REFERENCES audit_runs(id);

ALTER TABLE static_tool_results ADD CONSTRAINT fk_static_results_file
    FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE;
ALTER TABLE static_tool_results ADD CONSTRAINT fk_static_results_run
    FOREIGN KEY (run_id) REFERENCES audit_runs(id);

ALTER TABLE ingestor_rejections ADD CONSTRAINT fk_ingestor_rejections_run
    FOREIGN KEY (run_id) REFERENCES audit_runs(id) ON DELETE CASCADE;

-- ============================================================================
-- Indexes
-- ============================================================================

CREATE INDEX IF NOT EXISTS idx_projects_name ON projects(name);
CREATE INDEX IF NOT EXISTS idx_projects_repo_path ON projects(repo_path);

CREATE INDEX IF NOT EXISTS idx_audit_configs_project ON audit_configs(project_id);

CREATE INDEX IF NOT EXISTS idx_audit_runs_project ON audit_runs(project_id);
CREATE INDEX IF NOT EXISTS idx_audit_runs_status ON audit_runs(status);

CREATE INDEX IF NOT EXISTS idx_files_project_path ON files(project_id, path);
CREATE INDEX IF NOT EXISTS idx_files_hash ON files(project_id, hash);
CREATE INDEX IF NOT EXISTS idx_files_last_audited ON files(project_id, last_audited_hash);

CREATE INDEX IF NOT EXISTS idx_file_imports_file ON file_imports(file_id);
CREATE INDEX IF NOT EXISTS idx_file_imports_project ON file_imports(project_id);
CREATE INDEX IF NOT EXISTS idx_file_imports_type ON file_imports(project_id, import_type);
CREATE INDEX IF NOT EXISTS idx_file_imports_resolved ON file_imports(project_id, import_type) WHERE resolved_path IS NULL;

CREATE INDEX IF NOT EXISTS idx_dep_edges_source ON dependency_edges(source_file_id);
CREATE INDEX IF NOT EXISTS idx_dep_edges_target ON dependency_edges(target_file_id);
CREATE INDEX IF NOT EXISTS idx_dep_edges_project ON dependency_edges(project_id);

CREATE INDEX IF NOT EXISTS idx_file_staleness_file ON file_staleness(file_id);
CREATE INDEX IF NOT EXISTS idx_file_staleness_run ON file_staleness(run_id);

CREATE INDEX IF NOT EXISTS idx_findings_project ON findings(project_id, status);
CREATE INDEX IF NOT EXISTS idx_findings_file ON findings(file_id);
CREATE INDEX IF NOT EXISTS idx_findings_run ON findings(run_id);
CREATE INDEX IF NOT EXISTS idx_findings_severity ON findings(severity);

CREATE INDEX IF NOT EXISTS idx_arch_state_project ON architecture_state(project_id);

CREATE INDEX IF NOT EXISTS idx_static_results_file ON static_tool_results(file_id);
CREATE INDEX IF NOT EXISTS idx_static_results_run ON static_tool_results(run_id);

CREATE INDEX IF NOT EXISTS idx_ingestor_rejections_run ON ingestor_rejections(run_id);
