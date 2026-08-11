//! Tree-sitter language-pack boundary.

mod language;

pub use language::{detect_language, language_available};

/// The pinned `tree-sitter-language-pack` version, recorded with every
/// revision so a pack upgrade is visible as a manifest input, not a silent
/// re-parse.
///
/// `tree-sitter-language-pack` exposes no runtime `VERSION` constant, so
/// this string is manually duplicated from the exact pin
/// (`tree-sitter-language-pack = "=1.14.3"`) in `Cargo.toml`. Bumping the
/// dependency requires updating both together; the test below fails
/// loudly if they ever drift apart.
pub const PACK_VERSION: &str = "1.14.3";

#[cfg(test)]
mod tests {
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
            pinned_version, "=1.14.3",
            "Cargo.toml must keep an exact (`=`) pin on tree-sitter-language-pack"
        );
        assert_eq!(pinned_version.trim_start_matches('='), super::PACK_VERSION);
    }
}
