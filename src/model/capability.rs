use super::ParserAvailability;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportState {
    Supported,
    Unsupported { reason: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrammarQueryCapabilities {
    pub abi_version: Option<u32>,
    pub highlights: SupportState,
    pub injections: SupportState,
    pub locals: SupportState,
    pub indents: SupportState,
    pub folds: SupportState,
    pub tags: SupportState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceCapabilities {
    pub structures: SupportState,
    pub imports: SupportState,
    pub exports: SupportState,
    pub comments: SupportState,
    pub docstrings: SupportState,
    pub symbols: SupportState,
    pub diagnostics: SupportState,
    pub chunks: SupportState,
    pub raw_tree: SupportState,
    pub source_spans: SupportState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageCapabilityReport {
    pub language: String,
    pub detected: bool,
    pub known_to_pack: bool,
    pub parser: ParserAvailability,
    pub grammar_queries: GrammarQueryCapabilities,
    pub evidence: EvidenceCapabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_is_distinct_from_empty_results() {
        let state = SupportState::Unsupported {
            reason: "grammar does not ship tags query".into(),
        };
        assert_ne!(state, SupportState::Supported);
    }
}
