use serde::{Deserialize, Serialize};

/// Availability status of a language parser for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParserAvailability {
    /// A parser is installed and ready for this file type.
    Available,
    /// No parser exists for this file type.
    Unavailable,
    /// A parser exists for this file type but failed to initialize or load.
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

/// The result of attempting to parse a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseOutcome {
    /// Parsing was not attempted (e.g. parser unavailable or file skipped).
    NotAttempted,
    /// Parsing succeeded with no syntax errors.
    Succeeded,
    /// Parsing produced syntax errors.
    SyntaxErrors { count: u32 },
    /// Parsing failed catastrophically.
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

/// The completeness degree of structural evidence extracted from a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractionCompleteness {
    /// All structural and symbol evidence was extracted.
    Full,
    /// Extraction succeeded partially (e.g. truncated due to size or partial syntax errors).
    Partial,
    /// Only file content/hash was recorded; no AST evidence extracted.
    ContentOnly,
    /// Extraction was unavailable (no parser).
    Unavailable,
}

impl ExtractionCompleteness {
    #[must_use]
    /// The canonical `files`/`evidence` TEXT token for this value — the same
    /// strings the store's CHECK constraints enumerate.
    pub fn as_sql_text(&self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Partial => "Partial",
            Self::ContentOnly => "ContentOnly",
            Self::Unavailable => "Unavailable",
        }
    }
}

/// The origin or generator of an evidence record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceOrigin {
    /// Structural AST evidence from a language pack.
    PackStructure,
    /// Symbol definition/reference evidence from a language pack.
    PackSymbol,
    /// Direct raw tree-sitter AST capture.
    RawTree,
    /// Plain source file content evidence.
    SourceContent,
    /// Relationship derived across files or entities.
    DerivedRelationship,
    /// Evidence produced by SlugAudit's own generic tree-walk
    /// (e.g. `evidence::generic_imports`), not by the language pack's
    /// `process()`. Distinct from `PackStructure` so consumers can tell
    /// "the pack found this" from "our generic walker found this".
    GenericWalker,
    /// Evidence produced by the Rust call-graph walker.
    RustCallWalker,
}

/// Recorded outcome of executing a parser on a single file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserRun {
    /// Parser availability status when the file was processed.
    pub availability: ParserAvailability,
    /// Result outcome of the parse attempt.
    pub outcome: ParseOutcome,
    /// Degree of completeness of evidence extraction.
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
        assert_eq!(
            run.validate(),
            Err("an unavailable parser cannot report a parse outcome")
        );
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
