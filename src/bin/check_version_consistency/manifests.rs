//! Reads the hand-duplicated copies of the version and returns one failure
//! string per disagreement.
//!
//! A file that cannot be read is an `Err` (the gate cannot produce a verdict
//! and exits 2). A file that reads but is malformed — bad JSON, no version
//! key — is a normal `Ok(failures)` entry (exit 1): the defect is in the
//! manifest, not in the gate's ability to read it.

use std::fs;
use std::path::Path;

const CARGO_TOML: &str = "Cargo.toml";
const PLUGIN_MANIFEST: &str = "lhm.plugin.json";
const CHANGELOG: &str = "CHANGELOG.md";
const PARSE_MODULE: &str = "src/parse/mod.rs";
const PACK_DEPENDENCY: &str = "tree-sitter-language-pack";

/// Runs the three checks against `root`, returning their failures.
///
/// # Errors
///
/// Returns a message when any of the four source files cannot be read — a
/// gate that cannot read what it verifies must not print a verdict.
pub(super) fn check(root: &Path) -> Result<Vec<String>, String> {
    let cargo = read(root, CARGO_TOML)?;
    let plugin = read(root, PLUGIN_MANIFEST)?;
    let changelog = read(root, CHANGELOG)?;
    let parse = read(root, PARSE_MODULE)?;

    let mut failures = Vec::new();

    let cargo_version = cargo_version(&cargo);
    let plugin_version = match plugin_version(&plugin) {
        Ok(version) => Some(version),
        Err(error) => {
            failures.push(error);
            None
        }
    };
    match (&cargo_version, &plugin_version) {
        (Some(cargo), Some(plugin)) if cargo != plugin => failures.push(format!(
            "{CARGO_TOML} version `{cargo}` and {PLUGIN_MANIFEST} version `{plugin}` disagree"
        )),
        (None, _) => failures.push(format!("{CARGO_TOML} has no top-level `version = \"...\"`")),
        _ => {}
    }

    let newest = changelog_version(&changelog);
    match (&cargo_version, &newest) {
        (Some(cargo), Some(newest)) if cargo != newest => failures.push(format!(
            "newest {CHANGELOG} release `{newest}` and {CARGO_TOML} version `{cargo}` disagree"
        )),
        (Some(_), None) => {
            failures.push(format!("{CHANGELOG} has no released `## [x.y.z]` heading"))
        }
        _ => {}
    }

    match (&pack_pin(&cargo), &pack_version(&parse)) {
        (Some(pinned), Some(recorded)) => {
            if !pinned.starts_with('=') {
                failures.push(format!(
                    "{PACK_DEPENDENCY} pin `{pinned}` is not exact; it must be `=x.y.z`"
                ));
            } else if pinned.trim_start_matches('=') != recorded {
                failures.push(format!(
                    "{PACK_DEPENDENCY} pin `{pinned}` and parse::PACK_VERSION `{recorded}` \
                     disagree"
                ));
            }
        }
        (None, _) => failures.push(format!("{CARGO_TOML} has no `{PACK_DEPENDENCY}` pin")),
        (_, None) => failures.push(format!(
            "{PARSE_MODULE} declares no `pub const PACK_VERSION`"
        )),
    }

    Ok(failures)
}

fn read(root: &Path, relative: &str) -> Result<String, String> {
    let path = root.join(relative);
    fs::read_to_string(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// The text between the first pair of double quotes on a line.
fn quoted_value(line: &str) -> Option<String> {
    let mut parts = line.split('"');
    parts.next()?;
    parts.next().map(str::to_owned)
}

/// `Cargo.toml`'s `[package] version`: the first column-zero `version = "x"`
/// line. Dependency versions are indented or nested, so they cannot match.
pub(super) fn cargo_version(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.starts_with("version"))
        .and_then(quoted_value)
}

/// `lhm.plugin.json`'s top-level `version`.
pub(super) fn plugin_version(text: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("{PLUGIN_MANIFEST} is not valid JSON: {error}"))?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{PLUGIN_MANIFEST} has no string `version` field"))
}

/// The newest released heading in `CHANGELOG.md`: the first `## [x.y.z]`
/// line, skipping the `## [Unreleased]` section.
pub(super) fn changelog_version(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("## ["))
        .find(|rest| !rest.starts_with("Unreleased"))
        .and_then(|rest| rest.split(']').next())
        .map(str::to_owned)
}

/// `parse::PACK_VERSION`'s value.
pub(super) fn pack_version(source: &str) -> Option<String> {
    source
        .lines()
        .find(|line| line.trim_start().starts_with("pub const PACK_VERSION"))
        .and_then(quoted_value)
}

/// The `tree-sitter-language-pack` requirement string, e.g. `=1.14.3`.
pub(super) fn pack_pin(cargo: &str) -> Option<String> {
    cargo
        .lines()
        .find(|line| line.starts_with(PACK_DEPENDENCY))
        .and_then(quoted_value)
}
