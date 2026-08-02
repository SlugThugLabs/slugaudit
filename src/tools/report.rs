use super::context::{ensure_synced, open_verified_read_only};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Connection;
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
    let connection = open_verified_read_only(&synced)?;

    let response = build_report(&connection, synced.revision_id)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(Json(response))
}

fn build_report(
    connection: &Connection,
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

    let parser_failure_count: i64 = connection.query_row(
        "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND parser_availability != 'Available'",
        [],
        |row| row.get(0),
    )?;
    let open_finding_count: i64 = connection.query_row(
        "SELECT count(*) FROM findings WHERE status = 'current'",
        [],
        |row| row.get(0),
    )?;

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
