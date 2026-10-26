//! In-repo project configuration (`.planning/slugaudit/config.json`).

use crate::model::{AuditProfile, ResourceLimits, SQLITE_MAX_VALUE_BYTES, limits_for_profile};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors encountered while loading or parsing `.planning/slugaudit/config.json`.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read the config file from disk.
    #[error("failed to read config file at '{path}': {source}")]
    Io {
        /// Path to the configuration file.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Failed to deserialize configuration JSON.
    #[error("failed to parse config JSON at '{path}': {source}")]
    Parse {
        /// Path to the configuration file.
        path: PathBuf,
        /// Underlying JSON parse error.
        source: serde_json::Error,
    },
}

/// Errors encountered while writing `.planning/slugaudit/config.json`.
#[derive(Debug, Error)]
pub enum ConfigWriteError {
    /// The path is a symlink or not a regular file, so writing would follow a
    /// link out of the project or clobber a non-file. Nothing was written.
    #[error(
        "refusing to write config file at '{path}': it is a symlink or not a regular file; replace it with a regular file or remove it"
    )]
    NotRegularFile {
        /// Path to the configuration file.
        path: PathBuf,
    },
    /// Inspecting or writing the config file failed.
    #[error("failed to write config file at '{path}': {source}")]
    Io {
        /// Path to the configuration file.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
}

/// Optional per-project configuration overrides. An unknown key is a parse
/// error, so a misspelled limit is reported instead of silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Desired audit resource profile (`default`, `audit`, or `lean`).
    #[serde(default)]
    pub profile: Option<AuditProfile>,
    /// Maximum individual file size in bytes to index.
    #[serde(default)]
    pub max_file_bytes: Option<u64>,
    /// Largest file content in bytes a grammar is run on; larger files are
    /// stored but not parsed (`oversize_for_parse`). Clamped to the
    /// effective `max_file_bytes`. `0` is refused with a warning.
    #[serde(default)]
    pub max_parse_bytes: Option<u64>,
    /// Wall-clock allowance in milliseconds for every parse of one file; a
    /// file whose parse outlives it is stored without structure
    /// (`parse_timeout`) and the sync goes on. Capped by the sync budget.
    /// `0` is refused with a warning.
    #[serde(default)]
    pub max_parse_wall_clock_ms: Option<u64>,
    /// Longest `structure` query text in bytes; a longer query is refused
    /// before it is compiled. `0` is refused with a warning.
    #[serde(default)]
    pub max_structure_query_bytes: Option<usize>,
    /// Maximum cumulative size in bytes across all imported files.
    #[serde(default)]
    pub max_total_import_bytes: Option<u64>,
    /// Default `query`/`leads` page size in serialized JSON bytes; a request's
    /// `page_bytes` still overrides it, and the ceiling still clamps it.
    #[serde(default)]
    pub default_page_bytes: Option<usize>,
}

impl ProjectConfig {
    /// Attempts to load and parse `.planning/slugaudit/config.json` relative to `project_root`.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the file exists but cannot be read or parsed.
    pub fn load(project_root: &Path) -> Result<Option<Self>, ConfigError> {
        let config_path = project_root
            .join(".planning")
            .join("slugaudit")
            .join("config.json");
        let exists = config_path.try_exists().map_err(|source| ConfigError::Io {
            path: config_path.clone(),
            source,
        })?;
        if !exists {
            return Ok(None);
        }
        let content =
            crate::util::read_file_capped(&config_path, crate::util::MAX_FOREIGN_TEXT_BYTES)
                .map_err(|source| ConfigError::Io {
                    path: config_path.clone(),
                    source,
                })?;
        let cfg = serde_json::from_str::<Self>(&content).map_err(|source| ConfigError::Parse {
            path: config_path,
            source,
        })?;
        Ok(Some(cfg))
    }

    /// Writes `contents` to `.planning/slugaudit/config.json` under
    /// `project_root`.
    ///
    /// The exact path is inspected with `symlink_metadata` first: a symlink
    /// (to anywhere) or any non-regular file is refused, so the write can
    /// never land on a link target outside the project. A missing file is
    /// created. The check and the write are not atomic; that narrow race is
    /// accepted (`libc` is not a direct dependency, so `O_NOFOLLOW` is not
    /// used).
    ///
    /// # Errors
    ///
    /// Returns [`ConfigWriteError::NotRegularFile`] for a symlink or
    /// non-regular file, and [`ConfigWriteError::Io`] for any other failure.
    pub fn write_raw(project_root: &Path, contents: &str) -> Result<(), ConfigWriteError> {
        let path = project_root
            .join(".planning")
            .join("slugaudit")
            .join("config.json");
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(ConfigWriteError::NotRegularFile { path });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(ConfigWriteError::Io { path, source }),
        }
        std::fs::write(&path, contents).map_err(|source| ConfigWriteError::Io { path, source })
    }

    /// Loads project config if present and valid; falls back to default on error or absence.
    #[must_use]
    pub fn load_or_default(project_root: &Path) -> Self {
        match Self::load(project_root) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => Self::default(),
            // An I/O failure (unreadable file, a directory at the path, the
            // size cap) is not a parse failure; reporting it as "malformed"
            // would send the operator to the wrong fix.
            Err(ConfigError::Io { source, .. }) => {
                tracing::warn!(root_id = %crate::util::redacted_path(project_root), error_kind = "io", error = %source, "ignoring project config that could not be read; falling back to default");
                Self::default()
            }
            Err(ConfigError::Parse { source, .. }) => {
                tracing::warn!(root_id = %crate::util::redacted_path(project_root), error_kind = "parse", error = %source, "ignoring malformed project config; falling back to default");
                Self::default()
            }
        }
    }

    /// Overrides fields in `limits` with any explicitly specified options.
    #[must_use]
    pub fn apply_to_limits(&self, mut limits: ResourceLimits) -> ResourceLimits {
        if let Some(profile) = self.profile {
            limits = limits_for_profile(profile);
        }
        if let Some(file_bytes) = self.max_file_bytes {
            let capped = file_bytes.min(SQLITE_MAX_VALUE_BYTES);
            if capped != file_bytes {
                tracing::warn!(
                    requested = file_bytes,
                    cap = SQLITE_MAX_VALUE_BYTES,
                    "project max_file_bytes exceeds SQLite's maximum storable value; clamping",
                );
            }
            limits.max_file_bytes = capped;
        }
        if let Some(parse_bytes) =
            ResourceLimits::config_nonzero("config max_parse_bytes", self.max_parse_bytes)
        {
            limits.max_parse_bytes = parse_bytes;
        }
        if let Some(wall_clock_ms) = ResourceLimits::config_nonzero(
            "config max_parse_wall_clock_ms",
            self.max_parse_wall_clock_ms,
        ) {
            limits.max_parse_wall_clock_ms = wall_clock_ms;
        }
        if let Some(query_bytes) = ResourceLimits::config_nonzero(
            "config max_structure_query_bytes",
            self.max_structure_query_bytes,
        ) {
            limits.max_structure_query_bytes = query_bytes;
        }
        if let Some(import_bytes) = self.max_total_import_bytes {
            limits.max_total_import_bytes = import_bytes;
        }
        if let Some(page_bytes) = self.default_page_bytes {
            limits.default_page_bytes = page_bytes;
        }
        limits.clamp_parse_to_file_bytes();
        limits
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
