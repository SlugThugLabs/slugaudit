use serde::{Deserialize, Serialize};

/// Identity and content integrity tracking for a single source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    /// Relative path from the project root.
    pub relative_path: String,
    /// Cryptographic hash of the file contents.
    pub content_hash: String,
    /// Algorithm identifier used to compute `content_hash`.
    pub hash_algorithm: String,
}

impl SourceIdentity {
    /// Constructs a new [`SourceIdentity`] using the default BLAKE3 algorithm.
    #[must_use]
    pub fn new(relative_path: String, content_hash: String) -> Self {
        Self {
            relative_path,
            content_hash,
            hash_algorithm: "blake3-v1".into(),
        }
    }
}
