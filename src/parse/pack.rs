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

    /// `PACK_VERSION` is a manually duplicated string because the
    /// `tree-sitter-language-pack` crate exposes no runtime version
    /// constant to derive it from. This test is the guard against that
    /// duplication drifting from the real, exactly-pinned dependency
    /// version in `Cargo.toml` (a caret requirement there would let a
    /// minor/patch bump land silently and break the "parser upgrade
    /// forces re-analysis" guarantee `PACK_VERSION` exists to provide).
    /// If this fails, you bumped one without the other — update both.
    #[test]
    fn pack_version_matches_the_cargo_toml_pin() {
        let cargo_toml = include_str!("../../Cargo.toml");
        let dependency_line = cargo_toml
            .lines()
            .find(|line| line.trim_start().starts_with("tree-sitter-language-pack"))
            .expect("tree-sitter-language-pack dependency line in Cargo.toml");
        let pinned_version = dependency_line
            .split('"')
            .nth(1)
            .expect("quoted version requirement on the dependency line");
        assert_eq!(
            pinned_version, "=1.13.7",
            "Cargo.toml must keep an exact (`=`) pin on tree-sitter-language-pack"
        );
        assert_eq!(
            pinned_version.trim_start_matches('='),
            super::super::PACK_VERSION
        );
    }
}
