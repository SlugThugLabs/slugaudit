//! Tree-sitter language-pack boundary.

mod language;
mod pack;

pub use language::{detect_language, language_available};
pub use pack::{PackParseError, PackParseReport, parse_source};

/// The pinned `tree-sitter-language-pack` version, recorded with every
/// revision so a pack upgrade is visible as a manifest input, not a silent
/// re-parse.
///
/// `tree-sitter-language-pack` exposes no runtime `VERSION` constant, so
/// this string is manually duplicated from the exact pin
/// (`tree-sitter-language-pack = "=1.13.7"`) in `Cargo.toml`. Bumping the
/// dependency requires updating both together;
/// `parse::pack::tests::pack_version_matches_the_cargo_toml_pin` fails
/// loudly if they ever drift apart.
pub const PACK_VERSION: &str = "1.13.7";
