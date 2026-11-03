//! The live surface, derived from the server itself rather than restated.
//!
//! `server::tool_surface()` returns exactly what the generated `list_tools`
//! returns (`ToolRouter::list_all()`, sorted by name) and `resources::list()`
//! is exactly what `list_resources` returns, so this snapshot is the wire
//! content — it cannot disagree with the server by being maintained by hand.

use serde_json::Value;

/// `tools/list` + `resources/list`, as the JSON a client receives.
///
/// # Errors
///
/// Returns a serialization error if either surface cannot be rendered. Both
/// are plain `serde` types of owned values, so this is a cannot-happen guard
/// rather than a state the caller has to handle.
pub(super) fn build() -> Result<Value, serde_json::Error> {
    let tools = serde_json::to_value(slugaudit_mcp_rust::server::tool_surface())?;
    let resources = serde_json::to_value(slugaudit_mcp_rust::resources::list())?;
    Ok(serde_json::json!({ "tools": tools, "resources": resources }))
}

/// Number of advertised tools in a snapshot, for the `--record` report.
///
/// # Errors
///
/// Returns a message when the snapshot has no `tools` array.
pub(super) fn tool_count(snapshot: &Value) -> Result<usize, String> {
    entries(snapshot, "tools")
}

/// Number of advertised resources in a snapshot, for the `--record` report.
///
/// # Errors
///
/// Returns a message when the snapshot has no `resources` array.
pub(super) fn resource_count(snapshot: &Value) -> Result<usize, String> {
    entries(snapshot, "resources")
}

fn entries(snapshot: &Value, key: &str) -> Result<usize, String> {
    match snapshot.get(key).and_then(Value::as_array) {
        Some(array) => Ok(array.len()),
        None => Err(format!("the surface has no `{key}` array")),
    }
}
