//! The per-section queries that fill a `report` snapshot, and the assembly
//! that joins them. Split from `report.rs` so the request/response schema
//! (the MCP `outputSchema`) stays separate from the SQL that counts truth.
//!
//! A failure in any section propagates: a report is built from full truth or
//! not at all. The counts are evidence only — no score, no ranking, and no
//! claim about what any number means.

use super::report::{
    DependencyEdgeTotals, EvidenceKindCount, ImportResolutionCount, LanguageCount,
    ParserFailureFile, ReportResponse, build_dependency_edges as build_edge_totals,
};
use super::report_completeness::{CompletenessEnvelope, build as build_completeness};
use super::{CREDENTIAL_PATH_PREDICATE, PARSER_FAILURE_PREDICATE};
use rusqlite::Transaction;

/// Cap on how many parser-failure files to include in the report. Enough
/// to surface the pattern (e.g. "all failures are scratch files") without
/// letting a project with thousands of broken files bloat the response.
pub(super) const MAX_PARSER_FAILURE_FILES: usize = 20;

/// Everything the tree-wide section queries contribute to the response.
struct ScopeSections {
    file_count: i64,
    completeness: CompletenessEnvelope,
    languages: Vec<LanguageCount>,
    evidence_counts: Vec<EvidenceKindCount>,
    parser_failure_count: i64,
    parser_failure_files: Vec<ParserFailureFile>,
}

/// Everything the graph/evidence section queries contribute.
struct GraphSections {
    open_finding_count: i64,
    import_resolution: Vec<ImportResolutionCount>,
    dependency_edges: DependencyEdgeTotals,
    diagnostic_count: i64,
    unsupported_language_unresolved_count: i64,
    credential_pattern_file_count: i64,
}

/// Assembles the report from per-section queries — the ordering plus the
/// completion log, nothing else. Query order matches the historical
/// single-pass build so the first failing statement is unchanged.
pub(super) fn build_report(
    connection: &Transaction<'_>,
    revision_id: String,
) -> Result<ReportResponse, rusqlite::Error> {
    let scope = read_scope_sections(connection)?;
    let graph = read_graph_sections(connection)?;

    // Counts and the revision id only — never row content — attached to
    // whatever span `run_blocking` (src/server.rs) currently has entered.
    tracing::info!(
        target: "slugaudit_mcp_rust::tools::report",
        revision_id,
        file_count = scope.file_count,
        parser_failure_count = scope.parser_failure_count,
        open_finding_count = graph.open_finding_count,
        diagnostic_count = graph.diagnostic_count,
        "report built"
    );
    Ok(ReportResponse {
        revision_id,
        file_count: scope.file_count,
        completeness: scope.completeness,
        languages: scope.languages,
        evidence_counts: scope.evidence_counts,
        parser_failure_count: scope.parser_failure_count,
        parser_failure_files: scope.parser_failure_files,
        parser_failure_files_total: scope.parser_failure_count,
        open_finding_count: graph.open_finding_count,
        import_resolution: graph.import_resolution,
        dependency_edges: graph.dependency_edges,
        diagnostic_count: graph.diagnostic_count,
        unsupported_language_unresolved_count: graph.unsupported_language_unresolved_count,
        credential_pattern_file_count: graph.credential_pattern_file_count,
    })
}

/// The tree-wide counts: files, completeness, languages, evidence kinds and
/// parser failures.
fn read_scope_sections(connection: &Transaction<'_>) -> Result<ScopeSections, rusqlite::Error> {
    let file_count = scalar_count(connection, "SELECT count(*) FROM files")?;
    // The index cannot hold a non-UTF-8 path, so sync persists the set of
    // them in `unrepresentable_paths` (refreshed by every full discovery
    // and every incremental stat sweep); the count is a plain index read,
    // never a filesystem walk on this read path.
    let unrepresentable_path_count =
        scalar_count(connection, "SELECT count(*) FROM unrepresentable_paths")?;
    let completeness = build_completeness(connection, unrepresentable_path_count)?;
    let languages = grouped_language_counts(connection)?;
    let evidence_counts = grouped_evidence_counts(connection)?;
    let (parser_failure_count, parser_failure_files) = parser_failures(connection)?;
    Ok(ScopeSections {
        file_count,
        completeness,
        languages,
        evidence_counts,
        parser_failure_count,
        parser_failure_files,
    })
}

/// The default-scope findings and graph facts: open findings, import
/// resolution, edge totals, diagnostics, unmodeled import syntax, and
/// credential-shaped paths.
fn read_graph_sections(connection: &Transaction<'_>) -> Result<GraphSections, rusqlite::Error> {
    let open_finding_count = scalar_count(
        connection,
        "SELECT count(*) FROM findings WHERE status = 'current'",
    )?;
    let import_resolution = grouped_import_resolution(connection)?;
    // The same edges counted by direction — the zero-decision entry point
    // carries the totals so an agent does not have to write the COUNT to
    // get them; per-file traversal stays in the recipe resource.
    let dependency_edges = build_edge_totals(connection)?;
    // Total diagnostics — linter/compiler diagnostics extracted from source
    // files. Separate from parser failures: a file can parse fine but
    // still have diagnostics.
    let diagnostic_count = scalar_count(
        connection,
        "SELECT count(*) FROM evidence WHERE kind = 'Diagnostic'",
    )?;
    // Of the Unresolved edges, how many come from import syntax the
    // resolver can't parse — see the field doc on
    // `unsupported_language_unresolved_count`. The verdict is cached on
    // each edge at write time (`revision_edges::resolve_and_store` sets
    // `syntax_unmodeled`), so this is a plain SQL count — never a per-edge
    // re-extraction per report call (C11).
    let unsupported_language_unresolved_count = scalar_count(
        connection,
        "SELECT count(*) FROM dependency_edges \
         WHERE resolution_kind = 'Unresolved' AND syntax_unmodeled = 1",
    )?;
    // Credential-pattern files are indexed like any other file (see the
    // field doc); `lower(path)` keeps the match robust to case, and the
    // shared predicate lives in `tools::mod` so `leads` and `report` can
    // never disagree about the pattern list.
    let credential_pattern_file_count = scalar_count(
        connection,
        &format!("SELECT count(*) FROM files WHERE {CREDENTIAL_PATH_PREDICATE}"),
    )?;
    Ok(GraphSections {
        open_finding_count,
        import_resolution,
        dependency_edges,
        diagnostic_count,
        unsupported_language_unresolved_count,
        credential_pattern_file_count,
    })
}

/// One `SELECT count(*)` scalar.
fn scalar_count(connection: &Transaction<'_>, sql: &str) -> Result<i64, rusqlite::Error> {
    connection.query_row(sql, [], |row| row.get(0))
}

fn grouped_language_counts(
    connection: &Transaction<'_>,
) -> Result<Vec<LanguageCount>, rusqlite::Error> {
    let mut language_statement = connection.prepare(
        "SELECT language, count(*) FROM files \
         WHERE language IS NOT NULL GROUP BY language ORDER BY language",
    )?;
    let languages = language_statement
        .query_map([], |row| {
            Ok(LanguageCount {
                language: row.get(0)?,
                file_count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(language_statement);
    Ok(languages)
}

fn grouped_evidence_counts(
    connection: &Transaction<'_>,
) -> Result<Vec<EvidenceKindCount>, rusqlite::Error> {
    let mut evidence_statement =
        connection.prepare("SELECT kind, count(*) FROM evidence GROUP BY kind ORDER BY kind")?;
    let evidence_counts = evidence_statement
        .query_map([], |row| {
            Ok(EvidenceKindCount {
                kind: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(evidence_statement);
    Ok(evidence_counts)
}

/// Failures include load/availability problems *and* parses that ran and
/// then failed — both are "the AI cannot trust this file's structural
/// evidence" conditions. Returns the uncapped total for exactly the
/// predicate the capped list selects (so the response can state whether
/// the list is complete without a second scan), and the capped list
/// itself — ordered by path for determinism, so a project with thousands
/// of broken files doesn't bloat the report and the AI can still
/// distinguish noise (scratch files) from real source files.
fn parser_failures(
    connection: &Transaction<'_>,
) -> Result<(i64, Vec<ParserFailureFile>), rusqlite::Error> {
    let total = scalar_count(
        connection,
        &format!("SELECT count(*) FROM files WHERE {PARSER_FAILURE_PREDICATE}"),
    )?;
    let mut failure_stmt = connection.prepare(&format!(
        "SELECT path, parse_outcome, parse_error_reason, extraction_completeness, \
                parser_availability \
         FROM files WHERE {PARSER_FAILURE_PREDICATE} \
         ORDER BY path \
         LIMIT ?1"
    ))?;
    let files = failure_stmt
        .query_map([MAX_PARSER_FAILURE_FILES], |row| {
            Ok(ParserFailureFile {
                path: row.get(0)?,
                parse_outcome: row.get(1)?,
                parse_error_reason: row.get(2)?,
                extraction_completeness: row.get(3)?,
                parser_availability: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(failure_stmt);
    Ok((total, files))
}

/// Import resolution breakdown — the key asymmetry signal. A project
/// where most imports are External or Unresolved has a disconnected
/// import graph; dependency traversal won't help much there.
fn grouped_import_resolution(
    connection: &Transaction<'_>,
) -> Result<Vec<ImportResolutionCount>, rusqlite::Error> {
    let mut import_resolution_stmt = connection.prepare(
        "SELECT resolution_kind, count(*) \
         FROM dependency_edges \
         GROUP BY resolution_kind ORDER BY resolution_kind",
    )?;
    let import_resolution = import_resolution_stmt
        .query_map([], |row| {
            Ok(ImportResolutionCount {
                kind: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(import_resolution_stmt);
    Ok(import_resolution)
}
