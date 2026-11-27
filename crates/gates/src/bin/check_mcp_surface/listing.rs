//! Directory-listing (`lhm.plugin.json`) verification against `Cargo.toml`
//! and the live MCP tool surface.
//!
//! `AGENTS.md` requires directory listings to state only what shipped code
//! does. Checking `lhm.plugin.json` inside `check_mcp_surface` ensures its
//! `"version"` never drifts from `Cargo.toml` and its `"tools"` list never
//! silently misses an added tool or retains a removed one.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

const LISTING_FILE: &str = "lhm.plugin.json";
const MANIFEST_FILE: &str = "Cargo.toml";

/// Verifies `<root>/lhm.plugin.json` against `<root>/Cargo.toml` and `live`.
/// Returns an empty vector when the listing agrees with both, or human-readable
/// drift lines when anything disagrees.
pub(super) fn check(root: &Path, live: &Value) -> Vec<String> {
    let manifest_path = root.join(MANIFEST_FILE);
    let manifest_text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => {
            return vec![format!(
                "{MANIFEST_FILE}: cannot read {}: {error}",
                manifest_path.display()
            )];
        }
    };
    let listing_path = root.join(LISTING_FILE);
    let listing_text = match std::fs::read_to_string(&listing_path) {
        Ok(text) => text,
        Err(error) => {
            return vec![format!(
                "{LISTING_FILE}: cannot read {}: {error}",
                listing_path.display()
            )];
        }
    };
    check_contents(&manifest_text, &listing_text, live)
}

/// Pure comparison of `Cargo.toml` text, `lhm.plugin.json` text, and the live
/// MCP surface JSON.
pub(super) fn check_contents(manifest_text: &str, listing_text: &str, live: &Value) -> Vec<String> {
    let mut problems = Vec::new();

    let Some(cargo_version) = package_version(manifest_text) else {
        problems.push(format!(
            "{MANIFEST_FILE}: missing `[package]` `version = \"...\"`"
        ));
        return problems;
    };

    let listing: Value = match serde_json::from_str(listing_text) {
        Ok(value) => value,
        Err(error) => {
            problems.push(format!("{LISTING_FILE}: invalid JSON: {error}"));
            return problems;
        }
    };

    match listing.get("version").and_then(Value::as_str) {
        Some(version) if version == cargo_version => {}
        Some(version) => problems.push(format!(
            "{LISTING_FILE}.version: listing has {version:?}, {MANIFEST_FILE} has {cargo_version:?}"
        )),
        None => problems.push(format!("{LISTING_FILE}.version: missing string version")),
    }

    let Some(live_tools) = live.get("tools").and_then(Value::as_array) else {
        problems.push("live surface is missing `tools` array".to_owned());
        return problems;
    };
    let live_names: BTreeSet<&str> = live_tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();

    let Some(listing_tools) = listing.get("tools").and_then(Value::as_array) else {
        problems.push(format!("{LISTING_FILE}.tools: missing `tools` array"));
        return problems;
    };

    let mut seen_names: BTreeSet<&str> = BTreeSet::new();
    for (index, entry) in listing_tools.iter().enumerate() {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            problems.push(format!(
                "{LISTING_FILE}.tools[{index}]: missing string `name`"
            ));
            continue;
        };
        if name.trim().is_empty() {
            problems.push(format!("{LISTING_FILE}.tools[{index}]: empty tool `name`"));
            continue;
        }
        if !seen_names.insert(name) {
            problems.push(format!(
                "{LISTING_FILE}.tools[{name}]: duplicate tool entry"
            ));
        }
        let has_description = entry
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|desc| !desc.trim().is_empty());
        if !has_description {
            problems.push(format!(
                "{LISTING_FILE}.tools[{name}].description: missing or empty description"
            ));
        }
    }

    for live_name in &live_names {
        if !seen_names.contains(live_name) {
            problems.push(format!(
                "{LISTING_FILE}.tools[{live_name}]: advertised on live MCP surface, absent from {LISTING_FILE}"
            ));
        }
    }
    for listed_name in &seen_names {
        if !live_names.contains(listed_name) {
            problems.push(format!(
                "{LISTING_FILE}.tools[{listed_name}]: listed in {LISTING_FILE}, absent from live MCP surface"
            ));
        }
    }

    problems
}

/// Extracts `version = "..."` from the `[package]` table of `Cargo.toml`.
fn package_version(manifest_text: &str) -> Option<&str> {
    let mut in_package = false;
    for raw_line in manifest_text.lines() {
        let line = match raw_line.split_once('#') {
            Some((before_comment, _)) => before_comment.trim(),
            None => raw_line.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some(rhs) = line.strip_prefix("version")
            && let Some(quoted) = rhs.trim_start().strip_prefix('=')
            && let Some(unquoted_start) = quoted.trim_start().strip_prefix('"')
            && let Some((version, _)) = unquoted_start.split_once('"')
        {
            return Some(version);
        }
    }
    None
}
