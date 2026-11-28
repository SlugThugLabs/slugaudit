//! The per-section queries that fill a `report` snapshot: scope, parser
//! failures, and dependency-edge totals.

use super::completeness::build_completeness;
use super::sql::{count_where, grouped_column_counts, scalar_sql};
use super::{
    CompletenessEnvelope, DependencyEdgeTotals, EvidenceKindCount, ImportResolutionCount,
    LanguageCount, ParserFailureFile,
};
use crate::tools::{CREDENTIAL_PATH_PREDICATE, PARSER_FAILURE_PREDICATE};
use rusqlite::Transaction;

pub(super) const MAX_PARSER_FAILURE_FILES: usize = 20;

pub(super) struct ScopeSections {
    pub(super) file_count: i64,
    pub(super) completeness: CompletenessEnvelope,
    pub(super) languages: Vec<LanguageCount>,
    pub(super) evidence_counts: Vec<EvidenceKindCount>,
    pub(super) parser_failure_count: i64,
    pub(super) parser_failure_files: Vec<ParserFailureFile>,
}

pub(super) struct GraphSections {
    pub(super) import_resolution: Vec<ImportResolutionCount>,
    pub(super) dependency_edges: DependencyEdgeTotals,
    pub(super) diagnostic_count: i64,
    pub(super) unsupported_language_unresolved_count: i64,
    pub(super) credential_pattern_file_count: i64,
}

pub(super) struct ReportSections {
    pub(super) scope: ScopeSections,
    pub(super) graph: GraphSections,
}

pub(super) fn read_sections(
    connection: &Transaction<'_>,
) -> Result<ReportSections, rusqlite::Error> {
    let scope = read_scope_sections(connection)?;
    let graph = read_graph_sections(connection, &scope.evidence_counts)?;
    Ok(ReportSections { scope, graph })
}

fn read_scope_sections(conn: &Transaction<'_>) -> Result<ScopeSections, rusqlite::Error> {
    let file_count = count_where(conn, "files", None)?;
    let completeness = build_completeness(conn)?;
    let languages = grouped_column_counts(
        conn,
        "files",
        "language",
        Some("language IS NOT NULL"),
        |language, file_count| LanguageCount {
            tested: crate::parse::is_tested_language(&language),
            language,
            file_count,
        },
    )?;
    let evidence_counts = grouped_column_counts(conn, "evidence", "kind", None, |kind, count| {
        EvidenceKindCount { kind, count }
    })?;
    let (parser_failure_count, parser_failure_files) = parser_failures(conn)?;
    Ok(ScopeSections {
        file_count,
        completeness,
        languages,
        evidence_counts,
        parser_failure_count,
        parser_failure_files,
    })
}

fn read_graph_sections(
    conn: &Transaction<'_>,
    evidence_counts: &[EvidenceKindCount],
) -> Result<GraphSections, rusqlite::Error> {
    let import_resolution = grouped_column_counts(
        conn,
        "dependency_edges",
        "resolution_kind",
        None,
        |kind, count| ImportResolutionCount { kind, count },
    )?;
    let dependency_edges = DependencyEdgeTotals {
        outbound_edge_count: count_where(conn, "dependency_edges", None)?,
        inbound_edge_count: count_where(conn, "dependency_edges", Some("to_file_id IS NOT NULL"))?,
        outbound_file_count: scalar_sql(
            conn,
            "SELECT count(DISTINCT from_file_id) FROM dependency_edges",
        )?,
        inbound_file_count: scalar_sql(
            conn,
            "SELECT count(DISTINCT to_file_id) FROM dependency_edges",
        )?,
    };
    let diagnostic_count = evidence_counts
        .iter()
        .find(|entry| entry.kind == "Diagnostic")
        .map_or(0, |entry| entry.count);
    let unsupported_language_unresolved_count = count_where(
        conn,
        "dependency_edges",
        Some("resolution_kind = 'Unresolved' AND syntax_unmodeled = 1"),
    )?;
    let credential_pattern_file_count =
        count_where(conn, "files", Some(CREDENTIAL_PATH_PREDICATE))?;
    Ok(GraphSections {
        import_resolution,
        dependency_edges,
        diagnostic_count,
        unsupported_language_unresolved_count,
        credential_pattern_file_count,
    })
}

fn parser_failures(
    conn: &Transaction<'_>,
) -> Result<(i64, Vec<ParserFailureFile>), rusqlite::Error> {
    let total = count_where(conn, "files", Some(PARSER_FAILURE_PREDICATE))?;
    let mut stmt = conn.prepare(&format!(
        "SELECT path, parse_outcome, parse_error_reason, extraction_completeness, parser_availability \
         FROM files WHERE {PARSER_FAILURE_PREDICATE} ORDER BY path LIMIT ?1"
    ))?;
    let limit = i64::try_from(MAX_PARSER_FAILURE_FILES)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let files = stmt
        .query_map([limit], |row| {
            Ok(ParserFailureFile {
                path: row.get(0)?,
                parse_outcome: row.get(1)?,
                parse_error_reason: row.get(2)?,
                extraction_completeness: row.get(3)?,
                parser_availability: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    Ok((total, files))
}
