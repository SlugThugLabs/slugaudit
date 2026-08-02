use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessInput {
    pub contract_version: String,
    pub schema_version: String,
    pub parser_pack_version: String,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedRevision {
    pub revision_id: String,
    pub input: FreshnessInput,
}

impl VerifiedRevision {
    /// # Errors
    ///
    /// Returns an error if `revision_id` or `input.manifest_hash` is empty
    /// — a revision cannot be verified without both.
    pub fn new(revision_id: String, input: FreshnessInput) -> Result<Self, &'static str> {
        if revision_id.trim().is_empty() || input.manifest_hash.trim().is_empty() {
            return Err("revision identity and manifest hash are required");
        }
        Ok(Self { revision_id, input })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_requires_manifest_proof() {
        let input = FreshnessInput {
            contract_version: "1".into(),
            schema_version: "1".into(),
            parser_pack_version: "1.13.7".into(),
            manifest_hash: String::new(),
        };
        assert!(VerifiedRevision::new("rev-1".into(), input).is_err());
    }
}
