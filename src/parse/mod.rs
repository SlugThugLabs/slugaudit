//! Tree-sitter language-pack boundary.

pub(crate) mod fetch_gate;
mod language;

pub use language::{PROSE_LANGUAGE, detect_language, is_prose_language_name, language_available};

/// The pinned `tree-sitter-language-pack` version, recorded with every
/// revision so a pack upgrade is visible as a manifest input, not a silent
/// re-parse.
///
/// `tree-sitter-language-pack` exposes no runtime `VERSION` constant, so
/// this string is manually duplicated from the exact pin
/// (`tree-sitter-language-pack = "=1.18.0"`) in `Cargo.toml`. Bumping the
/// dependency requires updating both together; the repo-level
/// `check_version_consistency` gate fails loudly if they ever drift apart
/// (it replaced the unit test that used to guard this next to the const, so
/// the check reads the manifest at a chosen root instead of only this
/// crate's own compile-time `include_str!`).
pub const PACK_VERSION: &str = "1.18.0";
