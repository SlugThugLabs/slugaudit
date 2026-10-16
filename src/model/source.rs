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

/// Filesystem metadata for a indexed source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    /// Length of the file in bytes.
    pub byte_len: u64,
    /// Last modification timestamp in unix seconds, if available.
    pub modified_unix_seconds: Option<i64>,
}

/// Language identification details for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSelection {
    /// Name of the detected language.
    pub language: String,
    /// True if the language was automatically detected by extension/content.
    pub detected: bool,
}

/// Snapshot of a source file's identity, metadata, and language determination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    /// Identity and content hash.
    pub identity: SourceIdentity,
    /// Filesystem metadata.
    pub metadata: FileMetadata,
    /// Detected language, if determined.
    pub language: Option<LanguageSelection>,
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
