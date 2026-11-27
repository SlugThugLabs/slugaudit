//! Multi-source resource-limit resolution: combines built-in profile defaults,
//! `~/.slugthug/slugaudit/config.json`, `SLUGAUDIT_*` environment overrides,
//! and the model's per-project session profile.

use super::global_config::{ConfigError, GlobalConfig};
use crate::model::{
    AuditProfile, EnvConfigError, EnvOverrides, MemoryDetectError, OverrideSource, ResourceLimits,
    SystemMemoryInfo, detect_system_memory, env_overrides, limits_for_profile,
};
use std::path::Path;
use thiserror::Error;

/// Why the limits for a project could not be derived.
#[derive(Debug, Error)]
pub enum ProjectLimitsError {
    /// The global config is present but unreadable, malformed, or sets a `0` limit.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A `SLUGAUDIT_*` environment variable is set but invalid or `0`.
    #[error(transparent)]
    Env(#[from] EnvConfigError),
    /// The `adaptive` profile needs host memory that could not be detected.
    #[error("profile 'adaptive' cannot be sized ({0}); choose the profile 'lean' or 'audit'")]
    Memory(#[from] MemoryDetectError),
    /// Process state (session profiles or warning ledger) was poisoned.
    #[error(transparent)]
    Lock(#[from] crate::util::LockPoisoned),
}

/// Validates the global `config.json`, all `SLUGAUDIT_*` environment
/// variables, and host memory sizing for the startup profile before serving.
///
/// # Errors
///
/// Returns [`ProjectLimitsError`] if reading `config.json`, parsing any
/// `SLUGAUDIT_*` override, or detecting host memory for `adaptive` fails.
pub fn validate_startup_config() -> Result<ResourceLimits, ProjectLimitsError> {
    let global = GlobalConfig::load()?;
    let env = env_overrides()?;
    validate_startup_config_with(&global, &env, detect_system_memory)
}

pub(crate) fn validate_startup_config_with(
    global: &GlobalConfig,
    env: &EnvOverrides,
    detect: impl FnOnce() -> Result<SystemMemoryInfo, MemoryDetectError>,
) -> Result<ResourceLimits, ProjectLimitsError> {
    resolve_limits_with(global, env, None, detect).map_err(|error| match error {
        ProjectLimitsError::Memory(source) => {
            ProjectLimitsError::Env(EnvConfigError::MemoryUndetectable { source })
        }
        other => other,
    })
}

/// Resolves effective [`ResourceLimits`] for the project at `project_root`.
///
/// # Errors
///
/// Returns [`ProjectLimitsError`] if global config, session profile,
/// `SLUGAUDIT_*` overrides, or host memory detection fails.
pub fn effective_limits(project_root: &Path) -> Result<ResourceLimits, ProjectLimitsError> {
    let global = GlobalConfig::load()?;
    let session = super::session_profile::session_profile(project_root)?;
    let env = env_overrides()?;
    resolve_limits(&global, &env, session)
}

/// Combines built-in profile defaults, `global`, `env`, and `session_profile`.
pub(crate) fn resolve_limits(
    global: &GlobalConfig,
    env: &EnvOverrides,
    session_profile: Option<AuditProfile>,
) -> Result<ResourceLimits, ProjectLimitsError> {
    resolve_limits_with(global, env, session_profile, detect_system_memory)
}

fn resolve_limits_with(
    global: &GlobalConfig,
    env: &EnvOverrides,
    session_profile: Option<AuditProfile>,
    detect: impl FnOnce() -> Result<SystemMemoryInfo, MemoryDetectError>,
) -> Result<ResourceLimits, ProjectLimitsError> {
    global.validate_nonzero(None)?;
    let mut profile = AuditProfile::Adaptive;
    if let Some(chosen) = session_profile.or(env.profile).or(global.profile) {
        profile = chosen;
    }
    let mut limits = match profile {
        AuditProfile::Lean | AuditProfile::Audit => limits_for_profile(profile)?,
        AuditProfile::Adaptive => crate::model::adaptive_limits(detect()?),
    };
    global.apply_layer(&mut limits, OverrideSource::GlobalFile, false)?;
    env.apply_layer(
        &mut limits,
        OverrideSource::Environment,
        global.max_result_store_bytes.is_some(),
    )?;
    limits.clamp_parse_to_file_bytes()?;
    Ok(limits)
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
