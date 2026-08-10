use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub relative_path: String,
    pub content_hash: String,
    pub hash_algorithm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    pub byte_len: u64,
    pub modified_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSelection {
    pub language: String,
    pub detected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub identity: SourceIdentity,
    pub metadata: FileMetadata,
    pub language: Option<LanguageSelection>,
}

impl SourceIdentity {
    #[must_use]
    pub fn new(relative_path: String, content_hash: String) -> Self {
        Self {
            relative_path,
            content_hash,
            hash_algorithm: "blake3-v1".into(),
        }
    }
}
