// slugaudit-line-exception: approved-by=slugthug; reason=the persisted mechanics section is one format: its envelope check, per-entry read and read-modify-write share one shape and must not drift apart
//! Auditor-registered mechanics: the entry an auditor writes and the
//! versioned `mechanics` section of `.planning/slugaudit/config.json` that
//! persists entries for the project. The session set lives in
//! `session_mechanics`.
//!
//! This layer knows the JSON shape only. Whether an entry's query compiles,
//! whether its id collides with a built-in or with another entry, and what it
//! counts are decided in `tools::mechanic_catalog`, so a malformed entry is
//! reported there with its reason instead of being dropped here.

use super::config::{ConfigError, ConfigWriteError, ProjectConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The only `mechanics.version` this server reads and writes.
pub const MECHANICS_SECTION_VERSION: u64 = 1;

/// Most persisted entries one project's `config.json` may hold; entries past
/// it are reported invalid, and a further `persist` is refused.
pub const MAX_PERSISTED_MECHANICS: usize = 64;

/// One auditor-registered mechanic: a Tree-sitter query with exactly one
/// capture, run over the indexed files of `language`; every distinct node it
/// captures is one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicEntry {
    /// The id `leads` takes as `mechanic`; never a built-in's id.
    pub id: String,
    /// The grammar the query compiles against and the files it reads.
    pub language: String,
    /// Tree-sitter S-expression with exactly one capture.
    pub query: String,
    /// The author's own words for what the entry counts.
    pub note: String,
}

/// A persisted entry, or the whole section, that could not be read as an
/// entry. Reported, never dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPersistedEntry {
    /// Position in `mechanics.entries`; `None` when the whole section (or
    /// the file) is what failed.
    pub index: Option<usize>,
    /// The entry's `id` when it has a string one.
    pub id: Option<String>,
    /// Why it was not loaded.
    pub error: String,
}

/// The `mechanics` section as read: entries whose JSON shape is valid, with
/// their position, and everything that was not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedMechanics {
    /// `(index in mechanics.entries, entry)` in file order.
    pub entries: Vec<(usize, MechanicEntry)>,
    /// Entries (or the section) that failed to read, in file order.
    pub invalid: Vec<InvalidPersistedEntry>,
}

/// Why a `persist` or a persisted `remove` did not change `config.json`.
/// Nothing is written on any of these.
#[derive(Debug, Error)]
pub enum MechanicsWriteError {
    /// The existing `config.json` could not be read or is not JSON; it is
    /// not overwritten, so nothing it holds is lost.
    #[error("{0}; fix or remove it before persisting a mechanic")]
    Read(#[source] ConfigError),
    /// The existing `config.json` holds JSON that is not an object.
    #[error("config file at '{0}' is not a JSON object; fix it before persisting a mechanic")]
    NotAnObject(PathBuf),
    /// The existing `mechanics` section is not one this server can extend
    /// (a missing or future `version`, or an unexpected shape).
    #[error("the mechanics section of config.json cannot be updated: {0}")]
    Section(String),
    /// The project already holds [`MAX_PERSISTED_MECHANICS`] entries.
    #[error("config.json already holds {MAX_PERSISTED_MECHANICS} persisted mechanics, the cap")]
    Full,
    /// The updated file would be larger than the server reads back.
    #[error("config.json would be {bytes} bytes, over the {limit}-byte limit the server reads")]
    TooLarge {
        /// Encoded size of the updated file.
        bytes: usize,
        /// The read cap.
        limit: u64,
    },
    /// No persisted entry has the requested id.
    #[error("no persisted mechanic has id `{0}`")]
    NotFound(String),
    /// The symlink-safe write itself failed.
    #[error(transparent)]
    Write(#[from] ConfigWriteError),
}

/// Reads the persisted `mechanics` section of `project_root`'s
/// `config.json`. Never fails: an unreadable file, an unknown `version` or a
/// malformed entry becomes an [`InvalidPersistedEntry`], so the caller can
/// report it while the valid entries still load.
#[must_use]
pub fn load_persisted(project_root: &Path) -> PersistedMechanics {
    match read_config_value(project_root) {
        Ok(None) => PersistedMechanics::default(),
        Ok(Some(value)) => match value.get("mechanics") {
            None => PersistedMechanics::default(),
            Some(section) => parse_section(section),
        },
        Err(error) => {
            // The root is redacted and only the underlying cause is logged:
            // the error's own text names the config path.
            let (error_kind, cause) = match &error {
                ConfigError::Io { source, .. } => ("io", source.to_string()),
                ConfigError::Parse { source, .. } => ("parse", source.to_string()),
            };
            tracing::warn!(
                root_id = %crate::util::redacted_path(project_root),
                error_kind,
                error = %cause,
                "persisted mechanics could not be read; reporting the section invalid",
            );
            whole_section_invalid(error.to_string())
        }
    }
}

/// Appends `entry` to the persisted section, creating the section (and the
/// file) when absent and keeping every other key of `config.json`.
///
/// # Errors
///
/// See [`MechanicsWriteError`]; the file is unchanged on every error.
pub fn persist_entry(
    project_root: &Path,
    entry: &MechanicEntry,
) -> Result<(), MechanicsWriteError> {
    let encoded = serde_json::to_value(entry)
        .map_err(|error| MechanicsWriteError::Section(error.to_string()))?;
    modify_entries(project_root, |entries| {
        if entries.len() >= MAX_PERSISTED_MECHANICS {
            return Err(MechanicsWriteError::Full);
        }
        entries.push(encoded);
        Ok(())
    })
}

/// Removes every persisted entry whose `id` is `id` (valid or not), keeping
/// every other key of `config.json`.
///
/// # Errors
///
/// [`MechanicsWriteError::NotFound`] when no entry has that id; see
/// [`MechanicsWriteError`] for the rest. The file is unchanged on error.
pub fn remove_persisted(project_root: &Path, id: &str) -> Result<(), MechanicsWriteError> {
    modify_entries(project_root, |entries| {
        let before = entries.len();
        entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(id));
        if entries.len() == before {
            return Err(MechanicsWriteError::NotFound(id.to_owned()));
        }
        Ok(())
    })
}

fn config_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".planning")
        .join("slugaudit")
        .join("config.json")
}

/// The whole `config.json` as JSON, read under the same size cap
/// [`ProjectConfig::load`] uses; `None` when the file does not exist.
fn read_config_value(project_root: &Path) -> Result<Option<Value>, ConfigError> {
    let path = config_path(project_root);
    let exists = path.try_exists().map_err(|source| ConfigError::Io {
        path: path.clone(),
        source,
    })?;
    if !exists {
        return Ok(None);
    }
    let content = crate::util::read_file_capped(&path, crate::util::MAX_FOREIGN_TEXT_BYTES)
        .map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|source| ConfigError::Parse { path, source })
}

fn whole_section_invalid(error: String) -> PersistedMechanics {
    PersistedMechanics {
        entries: Vec::new(),
        invalid: vec![InvalidPersistedEntry {
            index: None,
            id: None,
            error,
        }],
    }
}

/// Checks the section's envelope (an object holding exactly `version` and
/// `entries`, at the version this server reads) and returns its entries.
fn section_entries(section: &Value) -> Result<&Vec<Value>, String> {
    let Some(object) = section.as_object() else {
        return Err("`mechanics` must be an object with `version` and `entries`".to_owned());
    };
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "version" | "entries"))
    {
        return Err(format!("unknown key `{key}` in `mechanics`"));
    }
    match object.get("version").and_then(Value::as_u64) {
        Some(MECHANICS_SECTION_VERSION) => {}
        Some(version) => {
            return Err(format!(
                "unsupported mechanics version {version}; this server reads version {MECHANICS_SECTION_VERSION}"
            ));
        }
        None => return Err("`mechanics.version` is missing or not an integer".to_owned()),
    }
    object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "`mechanics.entries` is missing or not an array".to_owned())
}

fn parse_section(section: &Value) -> PersistedMechanics {
    let entries = match section_entries(section) {
        Ok(entries) => entries,
        Err(error) => return whole_section_invalid(error),
    };
    let mut parsed = PersistedMechanics::default();
    for (index, raw) in entries.iter().enumerate() {
        let id = raw.get("id").and_then(Value::as_str).map(str::to_owned);
        if index >= MAX_PERSISTED_MECHANICS {
            parsed.invalid.push(InvalidPersistedEntry {
                index: Some(index),
                id,
                error: format!("over the cap of {MAX_PERSISTED_MECHANICS} persisted mechanics"),
            });
            continue;
        }
        match MechanicEntry::deserialize(raw) {
            Ok(entry) => parsed.entries.push((index, entry)),
            Err(error) => parsed.invalid.push(InvalidPersistedEntry {
                index: Some(index),
                id,
                error: format!("not a mechanic entry: {error}"),
            }),
        }
    }
    parsed
}

/// Read-modify-write of `mechanics.entries`: every other key of the file is
/// carried over untouched, the size is checked against the read cap, and the
/// write goes through [`ProjectConfig::write_raw`], which refuses a symlink
/// or non-regular `config.json`.
fn modify_entries(
    project_root: &Path,
    change: impl FnOnce(&mut Vec<Value>) -> Result<(), MechanicsWriteError>,
) -> Result<(), MechanicsWriteError> {
    let mut config = match read_config_value(project_root).map_err(MechanicsWriteError::Read)? {
        Some(Value::Object(object)) => object,
        Some(_) => return Err(MechanicsWriteError::NotAnObject(config_path(project_root))),
        None => Map::new(),
    };
    let mut entries = match config.get("mechanics") {
        Some(section) => section_entries(section)
            .map_err(MechanicsWriteError::Section)?
            .clone(),
        None => Vec::new(),
    };
    change(&mut entries)?;
    let mut section = Map::new();
    section.insert("version".to_owned(), Value::from(MECHANICS_SECTION_VERSION));
    section.insert("entries".to_owned(), Value::Array(entries));
    config.insert("mechanics".to_owned(), Value::Object(section));
    // `{:#}` is the pretty `Display` of a `Value`, which cannot fail.
    let encoded = format!("{:#}", Value::Object(config));
    let limit = crate::util::MAX_FOREIGN_TEXT_BYTES;
    if !u64::try_from(encoded.len()).is_ok_and(|bytes| bytes <= limit) {
        return Err(MechanicsWriteError::TooLarge {
            bytes: encoded.len(),
            limit,
        });
    }
    ProjectConfig::write_raw(project_root, &encoded)?;
    Ok(())
}

#[cfg(test)]
#[path = "mechanics_config_tests.rs"]
mod tests;
