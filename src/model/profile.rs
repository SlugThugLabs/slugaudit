//! Resource limit profiles (Lean, Audit, Adaptive).

use super::limits::{EvidenceLimits, ResourceLimits};
use super::system_ram::detect_system_memory;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
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
    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "lean" => Some(Self::Lean),
            "audit" => Some(Self::Audit),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }
}

#[must_use]
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
}
