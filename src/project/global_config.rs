//! Human global defaults (`~/.slugthug/slugaudit/config.json`): path lookup,
//! capped disk I/O, and load-time validation over [`crate::model::LimitOverrides`].
//!
//! Optional and read-only: tools never write it, and model-facing instructions
//! do not mention it.

use crate::model::LimitOverrides;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Human-configured global limit and profile defaults (`~/.slugthug/slugaudit/config.json`).
pub type GlobalConfig = LimitOverrides;

fn format_zero_location(path: Option<&Path>) -> String {
    match path {
        Some(path) => format!(" at '{}'", path.display()),
        None => String::new(),
    }
}

/// Errors encountered while loading or validating the global `config.json`.
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
    /// Failed to deserialize configuration JSON or encountered an unknown key.
    #[error("failed to parse config JSON at '{path}': {source}")]
    Parse {
        /// Path to the configuration file.
        path: PathBuf,
        /// Underlying JSON parse error.
        source: serde_json::Error,
    },
    /// A limit key in `config.json` is set to `0`, which is not a usable limit.
    #[error(
        "config limit `{key}`{} is set to 0, which is not a usable limit; set a positive value or remove it",
        format_zero_location(path.as_deref())
    )]
    Zero {
        /// Name of the offending JSON key.
        key: &'static str,
        /// Path to the configuration file when loaded from disk.
        path: Option<PathBuf>,
    },
}

#[cfg(not(test))]
fn global_config_path() -> Option<PathBuf> {
    crate::install::slugaudit_dir().map(|dir| dir.join("config.json"))
}

#[cfg(test)]
thread_local! {
    static TEST_CONFIG_PATH: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn global_config_path() -> Option<PathBuf> {
    TEST_CONFIG_PATH.with_borrow(Clone::clone)
}

#[cfg(test)]
pub(crate) fn set_global_config_path_for_test(path: Option<PathBuf>) {
    TEST_CONFIG_PATH.with_borrow_mut(|slot| *slot = path);
}

/// RAII test guard resetting the calling thread's `config.json` path on drop.
#[cfg(test)]
pub(crate) struct GlobalConfigGuardForTest;

#[cfg(test)]
impl GlobalConfigGuardForTest {
    pub(crate) fn none() -> Self {
        set_global_config_path_for_test(None);
        Self
    }
}

#[cfg(test)]
impl Drop for GlobalConfigGuardForTest {
    fn drop(&mut self) {
        set_global_config_path_for_test(None);
    }
}

impl LimitOverrides {
    /// Validates that no configured limit field in `self` is `0`.
    pub(crate) fn validate_nonzero(&self, path: Option<&Path>) -> Result<(), ConfigError> {
        if let Some(key) = self.first_zero_key() {
            return Err(ConfigError::Zero {
                key,
                path: path.map(Path::to_path_buf),
            });
        }
        Ok(())
    }

    // Owner decision (2026-10-31): the human can set base defaults; the model sets its own session settings through the tools and never needs to know this file exists.
    /// Loads `~/.slugthug/slugaudit/config.json`; returns [`Self::default`] when
    /// the file is absent or no home directory is resolved.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the file exists but cannot be read, parsed,
    /// or sets any limit to `0`.
    pub fn load() -> Result<Self, ConfigError> {
        let Some(path) = global_config_path() else {
            return Ok(Self::default());
        };
        let Some(cfg) = Self::load_from(&path)? else {
            return Ok(Self::default());
        };
        Ok(cfg)
    }

    /// Loads, parses, and validates the global config at `config_path`; returns
    /// `Ok(None)` when no file exists there.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if reading or parsing `config_path` fails, or if
    /// any configured limit is `0`.
    pub fn load_from(config_path: &Path) -> Result<Option<Self>, ConfigError> {
        let io_err = |source| ConfigError::Io {
            path: config_path.to_path_buf(),
            source,
        };
        if !config_path.try_exists().map_err(io_err)? {
            return Ok(None);
        }
        let content =
            crate::util::read_file_capped(config_path, crate::util::MAX_FOREIGN_TEXT_BYTES)
                .map_err(io_err)?;
        let cfg = serde_json::from_str::<Self>(&content).map_err(|source| ConfigError::Parse {
            path: config_path.to_path_buf(),
            source,
        })?;
        cfg.validate_nonzero(Some(config_path))?;
        Ok(Some(cfg))
    }
}

#[cfg(test)]
#[path = "global_config_tests.rs"]
pub(super) mod tests;
