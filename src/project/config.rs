// slugaudit-line-exception: approved-by=slugthug; reason=Project config load, write guard and limit mapping form one cohesive module.
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
        if let Some(import_bytes) = self.max_total_import_bytes {
            limits.max_total_import_bytes = import_bytes;
        }
        if let Some(page_bytes) = self.default_page_bytes {
            limits.default_page_bytes = page_bytes;
        }
        limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_capture::capture_warns;

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

    #[test]
    fn a_configured_default_page_size_overrides_the_profile_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf_dir = dir.path().join(".planning").join("slugaudit");
        std::fs::create_dir_all(&conf_dir).expect("create dir");
        std::fs::write(
            conf_dir.join("config.json"),
            r#"{"profile": "lean", "default_page_bytes": 4096}"#,
        )
        .expect("write");

        let cfg = ProjectConfig::load_or_default(dir.path());
        let applied = cfg.apply_to_limits(ResourceLimits::default());

        assert_eq!(applied.default_page_bytes, 4_096);
        assert_eq!(applied.max_query_response_bytes, 32 * 1024 * 1024);
    }

    #[test]
    fn a_misspelled_config_key_is_a_parse_error_not_silently_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conf_dir = dir.path().join(".planning").join("slugaudit");
        std::fs::create_dir_all(&conf_dir).expect("create dir");
        std::fs::write(conf_dir.join("config.json"), r#"{"max_file_byte": 5}"#).expect("write");

        let error = ProjectConfig::load(dir.path()).expect_err("unknown key must fail");

        assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
    }

    #[test]
    fn an_uninspectable_config_path_is_an_io_error_not_no_config() {
        let root = Path::new("/tmp").join("a".repeat(300));

        let error = ProjectConfig::load(&root).expect_err("stat failure must surface");

        assert!(matches!(error, ConfigError::Io { .. }), "{error}");
    }

    /// R6: an I/O failure (here a directory at the config path) is not a
    /// parse failure, and the log must send the operator to the right fix.
    #[test]
    fn an_unreadable_config_is_not_reported_as_malformed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = directory.path().join(".planning/slugaudit/config.json");
        std::fs::create_dir_all(config).expect("dir");
        let (_, logs) = capture_warns(|| ProjectConfig::load_or_default(directory.path()));
        assert!(
            logs.contains("could not be read") && !logs.contains("malformed"),
            "{logs}"
        );
    }

    #[test]
    fn a_config_file_above_sqlite_max_is_clamped() {
        let config = ProjectConfig {
            max_file_bytes: Some(2_000_000_000),
            ..Default::default()
        };

        let applied = config.apply_to_limits(ResourceLimits::default());

        assert_eq!(
            applied.max_file_bytes,
            crate::model::SQLITE_MAX_VALUE_BYTES,
            "SQLite cannot store a larger content value; the project limit is clamped"
        );
    }

    #[test]
    fn write_raw_refuses_a_directory_at_config_json() {
        let project = tempfile::tempdir().expect("temp dir");
        let config_path = project.path().join(".planning/slugaudit/config.json");
        std::fs::create_dir_all(&config_path).expect("directory in the file's place");

        let error = ProjectConfig::write_raw(project.path(), "{}")
            .expect_err("a directory must be refused");

        assert!(
            matches!(&error, ConfigWriteError::NotRegularFile { path } if *path == config_path),
            "{error:?}"
        );
        assert!(config_path.is_dir(), "the directory is left in place");
    }
}
