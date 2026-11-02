//! The human's global defaults (`~/.slugthug/slugaudit/config.json`) and the
//! one function that turns defaults, that file, the `SLUGAUDIT_*` environment
//! and the model's session profile into the limits a call runs under.
//!
//! The file is optional and read only: the tools never write it. There is no
//! per-repo config.

use crate::model::{
    AuditProfile, EnvConfigError, EnvOverrides, MemoryDetectError, ResourceLimits,
    SQLITE_MAX_VALUE_BYTES, env_overrides, limits_for_profile,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors encountered while loading or parsing the global `config.json`.
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
    /// Failed to deserialize configuration JSON, or the file names a key
    /// this build does not know.
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
    /// The global config is present but unreadable or malformed.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A `SLUGAUDIT_*` environment variable is set but invalid.
    #[error(transparent)]
    Env(#[from] EnvConfigError),
    /// The `adaptive` profile needs host memory that could not be detected;
    /// choose `lean` or `audit`.
    #[error("profile 'adaptive' cannot be sized ({0}); choose the profile 'lean' or 'audit'")]
    Memory(#[from] MemoryDetectError),
    /// Process state the limits are derived from (the session profiles or
    /// the warning ledger) was poisoned by a panic.
    #[error(transparent)]
    Lock(#[from] crate::util::LockPoisoned),
}

/// The human's global defaults. An unknown key is a parse error, so a
/// misspelled limit is reported instead of silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    /// Desired audit resource profile (`lean`, `audit`, or `adaptive`).
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
}

/// Where the global config lives: `<slugthug home>/slugaudit/config.json`,
/// resolved the way `install` resolves it. `None` when no home is known.
#[cfg(not(test))]
fn global_config_path() -> Option<PathBuf> {
    crate::install::slugaudit_dir().map(|dir| dir.join("config.json"))
}

// In tests the path comes from a per-thread seam and is absent by default, so
// no test reads the real home directory.
#[cfg(test)]
thread_local! {
    static TEST_CONFIG_PATH: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Where the global config lives in tests: the path the calling thread set,
/// or none.
#[cfg(test)]
fn global_config_path() -> Option<PathBuf> {
    TEST_CONFIG_PATH.with_borrow(Clone::clone)
}

/// Points the calling test thread's global config at `path` (`None`: no
/// global config).
#[cfg(test)]
pub(crate) fn set_global_config_path_for_test(path: Option<PathBuf>) {
    TEST_CONFIG_PATH.with_borrow_mut(|slot| *slot = path);
}

impl GlobalConfig {
    // Owner decision (2026-10-31): the human can set base defaults; the model sets its own session settings through the tools and never needs to know this file exists.
    /// Loads the global config; an absent file (or no known home) is the
    /// default config.
    ///
    /// # Errors
    ///
    /// A file that is present but unreadable (including a directory at the
    /// path or the size cap), malformed, or carrying an unknown key is an
    /// error carrying its path and cause, so the call fails instead of
    /// running with limits the file did not say.
    pub fn load() -> Result<Self, ConfigError> {
        let Some(path) = global_config_path() else {
            return Ok(Self::default());
        };
        match Self::load_from(&path)? {
            Some(config) => Ok(config),
            None => Ok(Self::default()),
        }
    }

    /// Attempts to load and parse the config at `config_path`; `None` when
    /// no file is there.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the file exists but cannot be read or parsed.
    pub fn load_from(config_path: &Path) -> Result<Option<Self>, ConfigError> {
        let exists = config_path.try_exists().map_err(|source| ConfigError::Io {
            path: config_path.to_path_buf(),
            source,
        })?;
        if !exists {
            return Ok(None);
        }
        let content =
            crate::util::read_file_capped(config_path, crate::util::MAX_FOREIGN_TEXT_BYTES)
                .map_err(|source| ConfigError::Io {
                    path: config_path.to_path_buf(),
                    source,
                })?;
        let cfg = serde_json::from_str::<Self>(&content).map_err(|source| ConfigError::Parse {
            path: config_path.to_path_buf(),
            source,
        })?;
        Ok(Some(cfg))
    }

    /// Overrides fields in `limits` with the limit keys this config sets.
    /// The `profile` key is not applied here: it only chooses the base
    /// limits, which [`effective_limits`] does before calling this.
    ///
    /// # Errors
    ///
    /// Returns [`crate::util::LockPoisoned`] when the ledger of limit
    /// warnings already logged was poisoned.
    pub fn apply_limit_keys(
        &self,
        mut limits: ResourceLimits,
    ) -> Result<ResourceLimits, crate::util::LockPoisoned> {
        if let Some(file_bytes) =
            ResourceLimits::config_nonzero("config max_file_bytes", self.max_file_bytes)?
        {
            let capped = file_bytes.min(SQLITE_MAX_VALUE_BYTES);
            if capped != file_bytes {
                tracing::warn!(
                    requested = file_bytes,
                    cap = SQLITE_MAX_VALUE_BYTES,
                    "config max_file_bytes exceeds SQLite's maximum storable value; clamping",
                );
            }
            limits.max_file_bytes = capped;
        }
        if let Some(parse_bytes) =
            ResourceLimits::config_nonzero("config max_parse_bytes", self.max_parse_bytes)?
        {
            limits.max_parse_bytes = parse_bytes;
        }
        if let Some(wall_clock_ms) = ResourceLimits::config_nonzero(
            "config max_parse_wall_clock_ms",
            self.max_parse_wall_clock_ms,
        )? {
            limits.max_parse_wall_clock_ms = wall_clock_ms;
        }
        if let Some(query_bytes) = ResourceLimits::config_nonzero(
            "config max_structure_query_bytes",
            self.max_structure_query_bytes,
        )? {
            limits.max_structure_query_bytes = query_bytes;
        }
        if let Some(import_bytes) = ResourceLimits::config_nonzero(
            "config max_total_import_bytes",
            self.max_total_import_bytes,
        )? {
            limits.max_total_import_bytes = import_bytes;
        }
        if let Some(page_bytes) =
            ResourceLimits::config_nonzero("config default_page_bytes", self.default_page_bytes)?
        {
            limits.default_page_bytes = page_bytes;
        }
        if let Some(store_bytes) = ResourceLimits::config_nonzero(
            "config max_result_store_bytes",
            self.max_result_store_bytes,
        )? {
            limits.max_result_store_bytes = store_bytes;
        }
        if let Some(grace_secs) = ResourceLimits::config_nonzero(
            "config result_active_grace_secs",
            self.result_active_grace_secs,
        )? {
            limits.result_active_grace_secs = grace_secs;
        }
        limits.clamp_parse_to_file_bytes()?;
        Ok(limits)
    }
}

/// The limits every tool call and sync for the project at `project_root`
/// runs under: the global file, the validated `SLUGAUDIT_*` environment and
/// this project's session profile, combined by `resolve_limits`, which
/// documents the precedence.
///
/// # Errors
///
/// Returns [`ProjectLimitsError`] when the global file is present but
/// unreadable, malformed or carries an unknown key, a `SLUGAUDIT_*` variable
/// is set but invalid, or the `adaptive` profile cannot be sized.
pub fn effective_limits(project_root: &Path) -> Result<ResourceLimits, ProjectLimitsError> {
    let global = GlobalConfig::load()?;
    let session = super::session_profile::session_profile(project_root)?;
    resolve_limits(&global, env_overrides()?, session)
}

/// Combines every source of limits.
///
/// Precedence, lowest to highest:
///
/// 1. built-in defaults: the limit table of the base profile (`adaptive`
///    unless a profile is chosen below);
/// 2. the global file (`~/.slugthug/slugaudit/config.json`): its limit keys;
/// 3. `SLUGAUDIT_*` environment variables: their limit overrides;
/// 4. the model's session profile (`project_control on` with `profile`),
///    held in memory for the project until the process exits.
///
/// The profile picks the base limit table, and the session profile beats
/// `SLUGAUDIT_PROFILE`, which beats the global file's `profile`. The global
/// file's limit keys are applied on that base, and the environment's limit
/// variables last, so choosing a profile never discards a limit a human set
/// in the file or the environment.
///
/// # Errors
///
/// Returns [`ProjectLimitsError::Memory`] when the chosen profile is
/// `adaptive` and the host memory cannot be detected, and
/// [`ProjectLimitsError::Lock`] when the warning ledger was poisoned.
pub(crate) fn resolve_limits(
    global: &GlobalConfig,
    env: &EnvOverrides,
    session_profile: Option<AuditProfile>,
) -> Result<ResourceLimits, ProjectLimitsError> {
    let base = match session_profile.or(env.profile).or(global.profile) {
        Some(profile) => limits_for_profile(profile)?,
        None => limits_for_profile(AuditProfile::default())?,
    };
    let mut limits = global.apply_limit_keys(base)?;
    env.apply(&mut limits, global.max_result_store_bytes.is_some())?;
    Ok(limits)
}

#[cfg(test)]
#[path = "global_config_tests.rs"]
mod tests;
