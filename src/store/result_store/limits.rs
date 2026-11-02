//! The limits a result store obeys.

/// The two limits the store obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultStoreLimits {
    /// Total bytes of admitted results (`max_result_store_bytes`).
    pub max_bytes: u64,
    /// Seconds an unfinished result stays protected after its last page
    /// (`result_active_grace_secs`).
    pub active_grace_secs: u64,
}

impl From<&crate::model::ResourceLimits> for ResultStoreLimits {
    fn from(limits: &crate::model::ResourceLimits) -> Self {
        Self {
            max_bytes: limits.max_result_store_bytes,
            active_grace_secs: limits.result_active_grace_secs,
        }
    }
}
