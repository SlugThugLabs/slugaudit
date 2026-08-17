use crate::model::{
    EvidenceItem, EvidenceKind, ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun,
};
use tree_sitter_language_pack::Error as PackError;

pub struct ParseResult {
    pub language: Option<String>,
    pub language_detected: bool,
    pub run: ParserRun,
    pub evidence: Vec<EvidenceItem>,
}

impl ParseResult {
    fn unavailable(language: Option<String>, language_detected: bool) -> Self {
        Self {
            language,
            language_detected,
            run: ParserRun {
                availability: ParserAvailability::Unavailable,
                outcome: ParseOutcome::NotAttempted,
                completeness: ExtractionCompleteness::Unavailable,
            },
            evidence: Vec::new(),
        }
    }
}

/// A parser that never loaded is a different failure than one that loaded
/// and then failed to parse — `ParserRun::validate` requires `LoadFailed`
/// to pair with `NotAttempted`, so these can't be conflated into one
/// `LoadFailed` bucket the way an untyped string representation could.
fn is_load_failure(error: &PackError) -> bool {
    matches!(
        error,
        PackError::LanguageNotFound(_)
            | PackError::DynamicLoad(_)
            | PackError::NullLanguagePointer(_)
            | PackError::ParserSetup(_)
    )
}

/// Detects a file's language and runs pack extraction when possible. Never
/// invents completeness: a file the pack can't identify or doesn't support
/// gets `Unavailable`/`NotAttempted`, never a false "parsed successfully".
pub fn analyze(relative_path: &str, content: Option<&str>) -> ParseResult {
    let Some(content) = content else {
        tracing::trace!(path = relative_path, "no content; parser not invoked");
        return ParseResult::unavailable(None, false);
    };
    let Some(language) = crate::parse::detect_language(relative_path) else {
        tracing::trace!(
            path = relative_path,
            "no language detected; parser not invoked"
        );
        return ParseResult::unavailable(None, false);
    };
    if !crate::parse::language_available(&language) {
        tracing::debug!(
            path = relative_path,
            language = %language,
            "detected language has no parser loaded in the pack; recording Unavailable",
        );
        return ParseResult::unavailable(Some(language), true);
    }
    match crate::evidence::extract(&language, content) {
        Ok(items) => {
            let diagnostic_count = items
                .iter()
                .filter(|item| item.kind == EvidenceKind::Diagnostic)
                .count();
            let outcome = if diagnostic_count > 0 {
                ParseOutcome::SyntaxErrors {
                    count: u32::try_from(diagnostic_count).unwrap_or(u32::MAX),
                }
            } else {
                ParseOutcome::Succeeded
            };
            tracing::debug!(
                path = relative_path,
                language = %language,
                diagnostic_count,
                outcome = ?outcome,
                "parser load + extraction succeeded",
            );
            ParseResult {
                language: Some(language),
                language_detected: true,
                run: ParserRun {
                    availability: ParserAvailability::Available,
                    outcome,
                    completeness: ExtractionCompleteness::Partial,
                },
                evidence: items,
            }
        }
        Err(error) if is_load_failure(&error) => {
            tracing::warn!(
                path = relative_path,
                language = %language,
                error = %error,
                "parser failed to load; recording LoadFailed",
            );
            ParseResult {
                language: Some(language),
                language_detected: true,
                run: ParserRun {
                    availability: ParserAvailability::LoadFailed {
                        reason: error.to_string(),
                    },
                    outcome: ParseOutcome::NotAttempted,
                    completeness: ExtractionCompleteness::Unavailable,
                },
                evidence: Vec::new(),
            }
        }
        Err(error) => {
            tracing::debug!(
                path = relative_path,
                language = %language,
                error = %error,
                "parser ran but extraction failed; recording Failed",
            );
            ParseResult {
                language: Some(language),
                language_detected: true,
                run: ParserRun {
                    availability: ParserAvailability::Available,
                    outcome: ParseOutcome::Failed {
                        reason: error.to_string(),
                    },
                    completeness: ExtractionCompleteness::Unavailable,
                },
                evidence: Vec::new(),
            }
        }
    }
}

#[cfg(test)]
#[path = "analyze_tests.rs"]
mod tests;
