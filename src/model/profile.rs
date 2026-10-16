//! Resource limit profiles (Lean, Audit, Adaptive).

use super::limits::{EvidenceLimits, ResourceLimits};
use super::system_ram::detect_system_memory;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
/// Named resource-limit presets an operator selects via `SLUGAUDIT_PROFILE`
/// or the project's `config.json`.
pub enum AuditProfile {
    /// Everyday coding assistant mode: fast, responsive, conservative budgets.
    Lean,
    /// Deep security audit and due-diligence mode: large file limits, uncapped AST blocks, generous time budgets.
    Audit,
    /// Adaptive mode: automatically scales limits based on host physical RAM and available memory.
    #[default]
    Adaptive,
}

impl AuditProfile {
    #[must_use]
    /// Case-insensitive parse of a profile name; anything unrecognized is `None`
    /// (`profile_from_env` reports the discarded setting).
    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "lean" => Some(Self::Lean),
            "audit" => Some(Self::Audit),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }
}

/// Resolves `SLUGAUDIT_PROFILE`, reporting why a set value is discarded.
///
/// An unset variable means the operator chose no profile — the default
/// applies and nothing needs saying. A variable that IS set but is not a
/// known profile (or cannot be read) is operator intent that would
/// otherwise vanish silently, so it warns — the same rule
/// `ResourceLimits::from_env` applies to the numeric overrides, and the
/// same rule AGENTS.md non-negotiable #3 states: reported, or it is a bug.
#[must_use]
pub(crate) fn profile_from_env() -> Option<AuditProfile> {
    match std::env::var("SLUGAUDIT_PROFILE") {
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => {
            tracing::warn!(
                variable = "SLUGAUDIT_PROFILE",
                "profile override could not be read; using the default profile",
            );
            None
        }
        Ok(raw) => match AuditProfile::parse_str(&raw) {
            Some(profile) => Some(profile),
            None => {
                tracing::warn!(
                    variable = "SLUGAUDIT_PROFILE",
                    "profile override is not a known profile; using the default profile",
                );
                None
            }
        },
    }
}

#[must_use]
/// The complete limit set for one profile. `Adaptive` scales the import
/// budget from detected host memory.
pub fn limits_for_profile(profile: AuditProfile) -> ResourceLimits {
    match profile {
        AuditProfile::Lean => ResourceLimits {
            max_file_bytes: 32 * 1024 * 1024,
            max_total_import_bytes: 2 * 1024 * 1024 * 1024,
            max_query_response_bytes: 32 * 1024 * 1024,
            max_query_sql_bytes: 50_000,
            max_query_vm_steps: 5_000_000,
            max_query_wall_clock: Duration::from_secs(10),
            max_query_value_bytes: 32 * 1024 * 1024,
            max_structure_query_bytes: 32_000,
            max_structure_matches: 2_000,
            max_structure_execution_time: Duration::from_secs(10),
            max_sync_wall_clock: Duration::from_secs(120),
            evidence: EvidenceLimits {
                max_items_per_file: 50_000,
                max_payload_bytes_per_item: 128 * 1024,
                max_payload_bytes_per_file: 32 * 1024 * 1024,
            },
        },
        AuditProfile::Audit => ResourceLimits {
            max_file_bytes: 256 * 1024 * 1024,
            max_total_import_bytes: 32 * 1024 * 1024 * 1024,
            max_query_response_bytes: 256 * 1024 * 1024,
            max_query_sql_bytes: 200_000,
            max_query_vm_steps: 50_000_000,
            max_query_wall_clock: Duration::from_secs(60),
            max_query_value_bytes: 256 * 1024 * 1024,
            max_structure_query_bytes: 128_000,
            max_structure_matches: 20_000,
            max_structure_execution_time: Duration::from_secs(60),
            max_sync_wall_clock: Duration::from_secs(1200),
            evidence: EvidenceLimits {
                max_items_per_file: 200_000,
                max_payload_bytes_per_item: 512 * 1024,
                max_payload_bytes_per_file: 256 * 1024 * 1024,
            },
        },
        AuditProfile::Adaptive => {
            let ram = detect_system_memory();
            let import_budget = (ram.available_bytes / 2).max(4 * 1024 * 1024 * 1024);
            let max_file = (ram.total_bytes / 10).clamp(256 * 1024 * 1024, 1024 * 1024 * 1024);
            ResourceLimits {
                max_file_bytes: max_file,
                max_total_import_bytes: import_budget,
                max_query_response_bytes: max_file as usize,
                max_query_sql_bytes: 200_000,
                max_query_vm_steps: 50_000_000,
                max_query_wall_clock: Duration::from_secs(60),
                max_query_value_bytes: max_file as usize,
                max_structure_query_bytes: 128_000,
                max_structure_matches: 20_000,
                max_structure_execution_time: Duration::from_secs(60),
                max_sync_wall_clock: Duration::from_secs(1200),
                evidence: EvidenceLimits {
                    max_items_per_file: 200_000,
                    max_payload_bytes_per_item: 512 * 1024,
                    max_payload_bytes_per_file: max_file as usize,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_str_recognizes_profiles() {
        assert_eq!(AuditProfile::parse_str("lean"), Some(AuditProfile::Lean));
        assert_eq!(AuditProfile::parse_str("AUDIT"), Some(AuditProfile::Audit));
        assert_eq!(
            AuditProfile::parse_str("adaptive"),
            Some(AuditProfile::Adaptive)
        );
        assert_eq!(AuditProfile::parse_str("unknown"), None);
    }

    #[test]
    fn profiles_provide_positive_limits() {
        for profile in [
            AuditProfile::Lean,
            AuditProfile::Audit,
            AuditProfile::Adaptive,
        ] {
            let limits = limits_for_profile(profile);
            assert!(limits.max_file_bytes > 0);
            assert!(limits.max_total_import_bytes >= limits.max_file_bytes);
            assert!(limits.max_query_response_bytes > 0);
        }
    }

    #[test]
    fn a_set_but_unknown_profile_warns_and_selects_nothing() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

        temp_env::with_var("SLUGAUDIT_PROFILE", Some("audut"), || {
            let (profile, logs) = crate::store::test_capture::capture_warns(profile_from_env);
            assert!(profile.is_none(), "the default profile applies");
            assert!(
                logs.contains("not a known profile"),
                "a discarded operator setting must be visible, or a typo'd \
                 profile silently does nothing:\n{logs}"
            );
        });
    }

    #[test]
    fn an_unset_profile_selects_nothing_without_warning() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

        temp_env::with_var_unset("SLUGAUDIT_PROFILE", || {
            let (profile, logs) = crate::store::test_capture::capture_warns(|| {
                tracing::warn!("control-line-probe-profile-unset");
                profile_from_env()
            });
            assert!(profile.is_none());
            assert!(
                logs.contains("control-line-probe-profile-unset"),
                "capture liveness control"
            );
            assert!(
                !logs.contains("profile override"),
                "an operator who set nothing must not be warned at:\n{logs}"
            );
        });
    }

    #[test]
    fn a_known_profile_resolves_without_warning() {
        let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");

        temp_env::with_var("SLUGAUDIT_PROFILE", Some("lean"), || {
            let (profile, logs) = crate::store::test_capture::capture_warns(|| {
                tracing::warn!("control-line-probe-profile-known");
                profile_from_env()
            });
            assert_eq!(profile, Some(AuditProfile::Lean));
            assert!(
                logs.contains("control-line-probe-profile-known"),
                "capture liveness control"
            );
            assert!(
                !logs.contains("profile override"),
                "a valid profile must not warn:\n{logs}"
            );
        });
    }
}
