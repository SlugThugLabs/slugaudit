//! In-repo project configuration (`.planning/slugaudit/config.json`).

use crate::model::{AuditProfile, ResourceLimits, limits_for_profile};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub profile: Option<AuditProfile>,
    #[serde(default)]
    pub max_file_bytes: Option<u64>,
    #[serde(default)]
    pub max_total_import_bytes: Option<u64>,
}

impl ProjectConfig {
    #[must_use]
    pub fn load_or_default(project_root: &Path) -> Self {
        let config_path = project_root
            .join(".planning")
            .join("slugaudit")
            .join("config.json");
        if let Ok(content) = std::fs::read_to_string(&config_path)
            && let Ok(cfg) = serde_json::from_str::<Self>(&content)
        {
            return cfg;
        }
        Self::default()
    }

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

        let cfg = ProjectConfig::load_or_default(dir.path());
        assert_eq!(cfg.profile, Some(AuditProfile::Audit));
        assert_eq!(cfg.max_file_bytes, Some(536_870_912));

        let base = ResourceLimits::default();
        let applied = cfg.apply_to_limits(base);
        assert_eq!(applied.max_file_bytes, 536_870_912);
        assert_eq!(applied.max_total_import_bytes, 32 * 1024 * 1024 * 1024);
    }
}
