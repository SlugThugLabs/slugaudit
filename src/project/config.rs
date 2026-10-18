//! In-repo project configuration (`.planning/slugaudit/config.json`).

use crate::model::{AuditProfile, ResourceLimits, limits_for_profile};
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

/// Optional per-project configuration overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Desired audit resource profile (`default`, `audit`, or `lean`).
    #[serde(default)]
    pub profile: Option<AuditProfile>,
    /// Maximum individual file size in bytes to index.
    #[serde(default)]
    pub max_file_bytes: Option<u64>,
    /// Maximum cumulative size in bytes across all imported files.
    #[serde(default)]
    pub max_total_import_bytes: Option<u64>,
}

impl ProjectConfig {
    /// Attempts to load and parse `.planning/slugaudit/config.json` relative to `project_root`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the file exists but cannot be read or parsed.
    pub fn load(project_root: &Path) -> Result<Option<Self>, ConfigError> {
        let config_path = project_root
            .join(".planning")
            .join("slugaudit")
            .join("config.json");
        if !config_path.exists() {
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

    /// Loads project config if present and valid; falls back to default on error or absence.
    #[must_use]
    pub fn load_or_default(project_root: &Path) -> Self {
        match Self::load(project_root) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => Self::default(),
            Err(err) => {
                let error_message = match &err {
                    ConfigError::Io { source, .. } => source.to_string(),
                    ConfigError::Parse { source, .. } => source.to_string(),
                };
                tracing::warn!(
                    root_id = %crate::util::redacted_path(project_root),
                    error = %error_message,
                    "ignoring malformed project config; falling back to default"
                );
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
            limits.max_file_bytes = file_bytes;
        }
        if let Some(import_bytes) = self.max_total_import_bytes {
            limits.max_total_import_bytes = import_bytes;
        }
        limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_valid_config_and_applies_overrides() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf_dir = dir.path().join(".planning").join("slugaudit");
        std::fs::create_dir_all(&conf_dir).expect("create dir");
        let json = r#"{"profile": "audit", "max_file_bytes": 536870912}"#;
        std::fs::write(conf_dir.join("config.json"), json).expect("write");

        let loaded = ProjectConfig::load(dir.path()).expect("load should succeed");
        assert!(loaded.is_some());

        let cfg = ProjectConfig::load_or_default(dir.path());
        assert_eq!(cfg.profile, Some(AuditProfile::Audit));
        assert_eq!(cfg.max_file_bytes, Some(536_870_912));

        let base = ResourceLimits::default();
        let applied = cfg.apply_to_limits(base);
        assert_eq!(applied.max_file_bytes, 536_870_912);
        assert_eq!(applied.max_total_import_bytes, 32 * 1024 * 1024 * 1024);
    }

    #[test]
    fn loads_default_when_missing_or_invalid() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(
            ProjectConfig::load(dir.path())
                .expect("load missing")
                .is_none()
        );
        let cfg = ProjectConfig::load_or_default(dir.path());
        assert!(cfg.profile.is_none());
        assert!(cfg.max_file_bytes.is_none());

        let conf_dir = dir.path().join(".planning").join("slugaudit");
        std::fs::create_dir_all(&conf_dir).expect("create dir");
        let conf_file = conf_dir.join("config.json");
        std::fs::write(&conf_file, "invalid json").expect("write");

        let load_err = ProjectConfig::load(dir.path()).expect_err("should return Parse error");
        match load_err {
            ConfigError::Parse { path, .. } => assert_eq!(path, conf_file),
            ConfigError::Io { .. } => panic!("expected Parse error, got Io"),
        }

        let cfg_invalid = ProjectConfig::load_or_default(dir.path());
        assert!(cfg_invalid.profile.is_none());

        let custom = ProjectConfig {
            max_total_import_bytes: Some(1024),
            ..Default::default()
        };
        let applied = custom.apply_to_limits(ResourceLimits::default());
        assert_eq!(applied.max_total_import_bytes, 1024);
    }
}
