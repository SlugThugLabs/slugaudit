use crate::model::{EvidenceItem, EvidenceKind};

pub struct ParseResult {
    pub language: Option<String>,
    pub language_detected: bool,
    pub parser_availability: &'static str,
    pub parse_outcome: &'static str,
    pub parse_error_reason: Option<String>,
    pub extraction_completeness: &'static str,
    pub evidence: Vec<EvidenceItem>,
}

impl ParseResult {
    fn unavailable(language: Option<String>, language_detected: bool) -> Self {
        Self {
            language,
            language_detected,
            parser_availability: "Unavailable",
            parse_outcome: "NotAttempted",
            parse_error_reason: None,
            extraction_completeness: "Unavailable",
            evidence: Vec::new(),
        }
    }
}

/// Detects a file's language and runs pack extraction when possible. Never
/// invents completeness: a file the pack can't identify or doesn't support
/// gets `Unavailable`/`NotAttempted`, never a false "parsed successfully".
pub fn analyze(relative_path: &str, content: Option<&str>) -> ParseResult {
    let Some(content) = content else {
        return ParseResult::unavailable(None, false);
    };
    let Some(language) = crate::parse::detect_language(relative_path) else {
        return ParseResult::unavailable(None, false);
    };
    if !crate::parse::language_available(&language) {
        return ParseResult::unavailable(Some(language), true);
    }
    match crate::evidence::extract(&language, content) {
        Ok(items) => {
            let has_errors = items
                .iter()
                .any(|item| item.kind == EvidenceKind::Diagnostic);
            ParseResult {
                language: Some(language),
                language_detected: true,
                parser_availability: "Available",
                parse_outcome: if has_errors {
                    "SyntaxErrors"
                } else {
                    "Succeeded"
                },
                parse_error_reason: None,
                extraction_completeness: "Partial",
                evidence: items,
            }
        }
        Err(error) => ParseResult {
            language: Some(language),
            language_detected: true,
            parser_availability: "LoadFailed",
            parse_outcome: "Failed",
            parse_error_reason: Some(error.to_string()),
            extraction_completeness: "Unavailable",
            evidence: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_no_detectable_language_is_marked_unavailable() {
        let result = analyze("README", Some("plain text"));
        assert_eq!(result.parser_availability, "Unavailable");
        assert_eq!(result.parse_outcome, "NotAttempted");
        assert!(!result.language_detected);
    }

    #[test]
    fn binary_content_is_never_analyzed() {
        let result = analyze("main.rs", None);
        assert_eq!(result.parser_availability, "Unavailable");
        assert!(result.evidence.is_empty());
    }

    #[test]
    fn a_supported_language_produces_real_evidence() {
        let result = analyze("lib.rs", Some("pub fn a() {}"));
        assert_eq!(result.language.as_deref(), Some("rust"));
        assert_eq!(result.parser_availability, "Available");
        assert_eq!(result.parse_outcome, "Succeeded");
        assert!(!result.evidence.is_empty());
    }
}
