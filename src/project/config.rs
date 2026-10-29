//! In-repo project configuration (`.planning/slugaudit/config.json`).

use crate::model::{
    AuditProfile, MemoryDetectError, ResourceLimits, SQLITE_MAX_VALUE_BYTES, limits_for_profile,
};
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

/// Why the limits for a project could not be derived.
#[derive(Debug, Error)]
pub enum ProjectLimitsError {
    /// The project config is present but unreadable or malformed.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The configured `adaptive` profile needs host memory that could not be
    /// detected; choose `lean` or `audit`.
    #[error(
        "config profile 'adaptive' cannot be sized ({0}); set the profile to 'lean' or 'audit'"
    )]
    Memory(#[from] MemoryDetectError),
}

/// Optional per-project configuration overrides. An unknown key is a parse
/// error, so a misspelled limit is reported instead of silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Desired audit resource profile (`default`, `audit`, or `lean`).
    #[serde(default)]
    pub profile: Option<AuditProfile>,
    /// Maximum individual file size in bytes to index. `0` is refused with a
    /// warning.
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
    /// Maximum cumulative size in bytes across all imported files. `0` is
    /// refused with a warning.
    #[serde(default)]
    pub max_total_import_bytes: Option<u64>,
    /// Default `query`/`leads` page size in serialized JSON bytes; a request's
    /// `page_bytes` still overrides it, and the ceiling still clamps it. `0` is
    /// refused with a warning.
    #[serde(default)]
    pub default_page_bytes: Option<usize>,
    /// Total bytes the result store keeps for paged results; `0` is refused
    /// with a warning.
    #[serde(default)]
    pub max_result_store_bytes: Option<u64>,
    /// Seconds an unfinished paged result stays protected from reclaim after
    /// its last page; `0` is refused with a warning.
    #[serde(default)]
    pub result_active_grace_secs: Option<u64>,
    /// The versioned section of auditor-persisted mechanics. Kept raw here
    /// so a malformed section never invalidates the limits above; it is read
    /// and reported entry by entry by `mechanics_config::load_persisted`.
    #[serde(default)]
    pub mechanics: Option<serde_json::Value>,
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

    /// Loads the project config; a missing file is the default config.
    /// First removes the temp files an interrupted update left beside it
    /// (`config_cleanup.rs`); that cleanup never fails the load.
    ///
    /// # Errors
    ///
    /// A config file that is present but unreadable (including a directory
    /// at the path or the size cap) or malformed is an error carrying its
    /// path and cause, so the call fails instead of running with limits the
    /// file did not say.
    pub fn load_or_default(project_root: &Path) -> Result<Self, ConfigError> {
        super::config_cleanup::remove_stale_temps(project_root);
        Ok(Self::load(project_root)?.unwrap_or_default())
    }

    /// The process `base` limits with `project_root`'s config applied: what
    /// every tool call and sync derives its budget from.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectLimitsError`] when the config is present but
    /// unreadable or malformed, or its `adaptive` profile cannot be sized.
    pub fn limits_for(
        project_root: &Path,
        base: ResourceLimits,
    ) -> Result<ResourceLimits, ProjectLimitsError> {
        Ok(Self::load_or_default(project_root)?.apply_to_limits(base)?)
    }

    /// Overrides fields in `limits` with any explicitly specified options.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryDetectError`] when the configured profile is
    /// `adaptive` and the host memory cannot be detected.
    pub fn apply_to_limits(
        &self,
        mut limits: ResourceLimits,
    ) -> Result<ResourceLimits, MemoryDetectError> {
        if let Some(profile) = self.profile {
            limits = limits_for_profile(profile)?;
        }
        if let Some(file_bytes) =
            ResourceLimits::config_nonzero("config max_file_bytes", self.max_file_bytes)
        {
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
        if let Some(import_bytes) = ResourceLimits::config_nonzero(
            "config max_total_import_bytes",
            self.max_total_import_bytes,
        ) {
            limits.max_total_import_bytes = import_bytes;
        }
        if let Some(page_bytes) =
            ResourceLimits::config_nonzero("config default_page_bytes", self.default_page_bytes)
        {
            limits.default_page_bytes = page_bytes;
        }
        if let Some(store_bytes) = ResourceLimits::config_nonzero(
            "config max_result_store_bytes",
            self.max_result_store_bytes,
        ) {
            limits.max_result_store_bytes = store_bytes;
        }
        if let Some(grace_secs) = ResourceLimits::config_nonzero(
            "config result_active_grace_secs",
            self.result_active_grace_secs,
        ) {
            limits.result_active_grace_secs = grace_secs;
        }
        limits.clamp_parse_to_file_bytes();
        Ok(limits)
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
