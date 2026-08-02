use crate::model::{ExtractionCompleteness, ParseOutcome, ParserAvailability, ParserRun};
use thiserror::Error;
use tree_sitter_language_pack::get_parser;

#[derive(Debug, Error)]
pub enum PackParseError {
    #[error("parser load failed: {0}")]
    Load(String),
    #[error("parser returned no syntax tree")]
    NoTree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackParseReport {
    pub language: String,
    pub root_kind: String,
    pub has_syntax_errors: bool,
    pub run: ParserRun,
}

/// # Errors
///
/// Returns an error if the pack cannot load a parser for `language`, or if
/// the parser returns no syntax tree at all.
pub fn parse_source(language: &str, source: &str) -> Result<PackParseReport, PackParseError> {
    let mut parser =
        get_parser(language).map_err(|error| PackParseError::Load(error.to_string()))?;
    let tree = parser.parse(source).ok_or(PackParseError::NoTree)?;
    let has_syntax_errors = tree.root_node().has_error();
    let outcome = if has_syntax_errors {
        ParseOutcome::SyntaxErrors { count: 1 }
    } else {
        ParseOutcome::Succeeded
    };
    Ok(PackParseReport {
        language: language.to_owned(),
        root_kind: tree.root_node().kind(),
        has_syntax_errors,
        run: ParserRun {
            availability: ParserAvailability::Available,
            outcome,
            completeness: ExtractionCompleteness::Partial,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_through_the_language_pack() {
        let report = parse_source("rust", "fn main() {}").expect("rust parser");
        assert_eq!(report.root_kind, "source_file");
        assert!(!report.has_syntax_errors);
    }
}
