//! Tree-sitter language-pack boundary.

mod language;
mod pack;

pub use language::{detect_language, language_available};
pub use pack::{PackParseError, PackParseReport, parse_source};

/// The pinned `tree-sitter-language-pack` version, recorded with every
/// revision so a pack upgrade is visible as a manifest input, not a silent
/// re-parse.
pub const PACK_VERSION: &str = "1.13.7";
