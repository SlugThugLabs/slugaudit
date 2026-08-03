use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParserAvailability {
    Available,
    Unavailable,
    LoadFailed { reason: String },
}

impl ParserAvailability {
    /// The stable text tag stored in `files.parser_availability`. Variant
    /// payloads (e.g. `LoadFailed`'s `reason`) live in their own column —
    /// see `ParserRun::error_reason`.
    #[must_use]
    pub fn as_sql_text(&self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Unavailable => "Unavailable",
            Self::LoadFailed { .. } => "LoadFailed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseOutcome {
    NotAttempted,
    Succeeded,
    SyntaxErrors { count: u32 },
    Failed { reason: String },
}

impl ParseOutcome {
    /// The stable text tag stored in `files.parse_outcome`. `SyntaxErrors`'s
    /// count is not persisted here — it's always derivable from
    /// `evidence` rows with `kind = 'Diagnostic'`, so storing it twice
    /// would just be a second place for it to drift out of sync.
    #[must_use]
    pub fn as_sql_text(&self) -> &'static str {
        match self {
            Self::NotAttempted => "NotAttempted",
            Self::Succeeded => "Succeeded",
            Self::SyntaxErrors { .. } => "SyntaxErrors",
            Self::Failed { .. } => "Failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionCompleteness {
    Full,
    Partial,
    ContentOnly,
    Unavailable,
}

impl ExtractionCompleteness {
    #[must_use]
    pub fn as_sql_text(&self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Partial => "Partial",
            Self::ContentOnly => "ContentOnly",
            Self::Unavailable => "Unavailable",
        }
    }
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

    /// The single failure-reason text to persist alongside this run, if
    /// either half of it carries one. `LoadFailed`/`Failed` are mutually
    /// exclusive by `validate`'s own rule, so there's never a conflict to
    /// resolve between them.
    #[must_use]
    pub fn error_reason(&self) -> Option<String> {
        match (&self.availability, &self.outcome) {
            (ParserAvailability::LoadFailed { reason }, _)
            | (_, ParseOutcome::Failed { reason }) => Some(reason.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_resource_and_parse_outcome_are_independent() {
        let run = ParserRun {
            availability: ParserAvailability::Available,
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

    #[test]
    fn sql_text_ignores_variant_payloads() {
        assert_eq!(
            ParserAvailability::LoadFailed { reason: "x".into() }.as_sql_text(),
            "LoadFailed"
        );
        assert_eq!(
            ParseOutcome::Failed { reason: "x".into() }.as_sql_text(),
            "Failed"
        );
    }

    #[test]
    fn error_reason_prefers_load_failure_over_parse_failure() {
        let run = ParserRun {
            availability: ParserAvailability::LoadFailed {
                reason: "load".into(),
            },
            outcome: ParseOutcome::NotAttempted,
            completeness: ExtractionCompleteness::Unavailable,
        };
        assert_eq!(run.error_reason().as_deref(), Some("load"));
    }

    #[test]
    fn error_reason_is_none_when_nothing_failed() {
        let run = ParserRun {
            availability: ParserAvailability::Available,
            outcome: ParseOutcome::Succeeded,
            completeness: ExtractionCompleteness::Partial,
        };
        assert_eq!(run.error_reason(), None);
    }
}
