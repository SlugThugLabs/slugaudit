use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParserAvailability {
    Available,
    Cached,
    Downloaded,
    Unavailable,
    LoadFailed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseOutcome {
    NotAttempted,
    Succeeded,
    SyntaxErrors { count: u32 },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionCompleteness {
    Full,
    Partial,
    ContentOnly,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceOrigin {
    PackStructure,
    PackSymbol,
    RawTree,
    SourceContent,
    DerivedRelationship,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserRun {
    pub availability: ParserAvailability,
    pub outcome: ParseOutcome,
    pub completeness: ExtractionCompleteness,
}

impl ParserRun {
    /// # Errors
    ///
    /// Returns an error if `availability` and `outcome` contradict each
    /// other (e.g. an unavailable parser reporting a parse outcome), or if
    /// `outcome` claims success while `completeness` claims no extraction
    /// happened.
    pub fn validate(&self) -> Result<(), &'static str> {
        let unavailable = matches!(
            self.availability,
            ParserAvailability::Unavailable | ParserAvailability::LoadFailed { .. }
        );
        if unavailable && !matches!(self.outcome, ParseOutcome::NotAttempted) {
            return Err("an unavailable parser cannot report a parse outcome");
        }
        if matches!(self.outcome, ParseOutcome::Succeeded)
            && matches!(self.completeness, ExtractionCompleteness::Unavailable)
        {
            return Err("successful parsing cannot have unavailable extraction");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_resource_and_parse_outcome_are_independent() {
        let run = ParserRun {
            availability: ParserAvailability::Cached,
            outcome: ParseOutcome::SyntaxErrors { count: 1 },
            completeness: ExtractionCompleteness::Partial,
        };
        assert!(run.validate().is_ok());
    }

    #[test]
    fn unavailable_parser_cannot_look_successful() {
        let run = ParserRun {
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::Succeeded,
            completeness: ExtractionCompleteness::Full,
        };
        assert!(run.validate().is_err());
    }
}
