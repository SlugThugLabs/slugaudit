use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceLimits {
    pub max_items_per_file: usize,
    pub max_payload_bytes_per_item: usize,
    pub max_payload_bytes_per_file: usize,
}

impl Default for EvidenceLimits {
    fn default() -> Self {
        Self {
            max_items_per_file: 10_000,
            max_payload_bytes_per_item: 64 * 1024,
            max_payload_bytes_per_file: 4 * 1024 * 1024,
        }
    }
}
