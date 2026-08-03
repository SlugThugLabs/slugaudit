use super::context::{ensure_synced, with_verified_read};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReportRequest {
    /// Any path inside the active project: its root, or a file/directory under it.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LanguageCount {
    pub language: String,
    pub file_count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EvidenceKindCount {
    pub kind: String,
    pub count: i64,
}

/// Automatic project snapshot: what evidence exists, not what's suspicious
/// about it. No score, no risk leads, no safe/unsafe claim.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportResponse {
    pub revision_id: String,
    pub file_count: i64,
    pub languages: Vec<LanguageCount>,
    pub evidence_counts: Vec<EvidenceKindCount>,
    pub parser_failure_count: i64,
    pub open_finding_count: i64,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project, or if
/// syncing or querying the project's database fails.
pub fn report(request: &Parameters<ReportRequest>) -> Result<Json<ReportResponse>, ErrorData> {
    let synced = ensure_synced(&request.0.path)?;
    let revision_id = synced.revision_id.clone();
    let response = with_verified_read(&synced, |tx| {
        build_report(tx, revision_id.clone())
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })?;
    Ok(Json(response))
}

fn build_report(
    connection: &Transaction<'_>,
    revision_id: String,
) -> Result<ReportResponse, rusqlite::Error> {
    let file_count: i64 =
        connection.query_row("SELECT count(*) FROM files", [], |row| row.get(0))?;

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

    // Failures include load/availability problems *and* parses that ran
    // and then failed — both are "the AI cannot trust this file's
    // structural evidence" conditions.
    let parser_failure_count: i64 = connection.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND (\
            parser_availability != 'Available' OR parse_outcome = 'Failed'\
         )",
        [],
        |row| row.get(0),
    )?;
    let open_finding_count: i64 = connection.query_row(
        "SELECT count(*) FROM findings WHERE status = 'current'",
        [],
        |row| row.get(0),
    )?;

    // Counts and the revision id only — never row content — attached to
    // whatever span `run_blocking` (src/server.rs) currently has entered.
    tracing::info!(
        revision_id,
        file_count,
        parser_failure_count,
        open_finding_count,
        "report built"
    );
    Ok(ReportResponse {
        revision_id,
        file_count,
        languages,
        evidence_counts,
        parser_failure_count,
        open_finding_count,
    })
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
