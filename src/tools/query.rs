use super::context::ensure_synced;
use crate::store;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Row;
use rusqlite::types::Value as SqlValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_ROWS: usize = 500;
const MAX_QUERY_LENGTH: usize = 10_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryRequest {
    /// Any path inside the active project.
    pub path: String,
    /// Read-only SQL against the project's own database — arbitrary joins,
    /// CTEs, and window functions are fine. Only writes are rejected, and
    /// that comes from the connection itself, not from inspecting this
    /// text: there is no keyword blocklist or table allowlist.
    pub sql: String,
}

/// Arbitrary read-only query results: one JSON object per row, column
/// names as keys. This is the general-purpose tool — search, symbol/
/// import/diagnostic lookup, dependency traversal, and source retrieval
/// all reach through it as ordinary `SELECT`s against the schema.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QueryResponse {
    pub revision_id: String,
    pub rows: Vec<serde_json::Value>,
    pub truncated: bool,
}

/// # Errors
///
/// Returns an error if `request.path` isn't an active project, `sql` is
/// empty or too long, the query fails to parse or execute (including any
/// attempted write, which SQLite itself rejects on this connection), or a
/// result value can't be represented.
pub fn query(request: &Parameters<QueryRequest>) -> Result<Json<QueryResponse>, ErrorData> {
    let QueryRequest { path, sql } = &request.0;
    let trimmed = sql.trim().trim_end_matches(';');
    if trimmed.is_empty() {
        return Err(ErrorData::invalid_params("sql must not be empty", None));
    }
    if sql.len() > MAX_QUERY_LENGTH {
        return Err(ErrorData::invalid_params(
            format!("sql exceeds {MAX_QUERY_LENGTH} characters"),
            None,
        ));
    }

    let synced = ensure_synced(path)?;
    let connection = store::open_read_only(&synced.database_path)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

    // A hard cap regardless of what the caller asks for, plus one extra row
    // so truncation is exact rather than guessed at.
    let wrapped = format!("SELECT * FROM ({trimmed}) LIMIT {}", MAX_ROWS + 1);
    let mut statement = connection
        .prepare(&wrapped)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let column_names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();

    let mut rows = statement
        .query_map([], |row| row_to_json(row, &column_names))
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;

    let truncated = rows.len() > MAX_ROWS;
    rows.truncate(MAX_ROWS);

    Ok(Json(QueryResponse {
        revision_id: synced.revision_id,
        rows,
        truncated,
    }))
}

fn row_to_json(row: &Row, column_names: &[String]) -> rusqlite::Result<serde_json::Value> {
    let mut object = serde_json::Map::with_capacity(column_names.len());
    for (index, name) in column_names.iter().enumerate() {
        let value: SqlValue = row.get(index)?;
        object.insert(name.clone(), sql_value_to_json(&value));
    }
    Ok(serde_json::Value::Object(object))
}

fn sql_value_to_json(value: &SqlValue) -> serde_json::Value {
    match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(number) => serde_json::Value::from(*number),
        SqlValue::Real(number) => serde_json::Number::from_f64(*number)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        SqlValue::Text(text) => serde_json::Value::String(text.clone()),
        SqlValue::Blob(bytes) => serde_json::Value::String(hex_encode(bytes)),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
