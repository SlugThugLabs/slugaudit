//! Tree-sitter language-pack boundary.

pub(crate) mod budget;
pub(crate) mod fetch_gate;
mod language;

pub use language::{
    PROSE_LANGUAGE, TESTED_LANGUAGES, detect_language, is_prose_language_name, is_tested_language,
    language_available,
};

/// The pinned `tree-sitter-language-pack` version, recorded with every
/// revision so a pack upgrade is visible as a manifest input, not a silent
/// re-parse.
///
/// `tree-sitter-language-pack` exposes no runtime `VERSION` constant, so
/// this string is manually duplicated from the exact pin
/// (`tree-sitter-language-pack = "=1.18.0"`) in `Cargo.toml`. Bumping the
/// dependency requires updating both together; the unit test in this module
/// fails if they drift apart.
pub const PACK_VERSION: &str = "1.18.0";

#[cfg(test)]
mod tests {
    use super::PACK_VERSION;
    use slugaudit_test_check::{Check, TestResult};

    const PIN_PREFIX: &str = "tree-sitter-language-pack = \"=";

    #[test]
    fn pack_version_matches_the_manifest_pin() -> TestResult {
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        let pin = manifest
            .lines()
            .find(|line| line.starts_with(PIN_PREFIX))
            .check("Cargo.toml must pin tree-sitter-language-pack exactly as `=x.y.z`")?;
        let version = pin
            .strip_prefix(PIN_PREFIX)
            .and_then(|rest| rest.split_once('"'))
            .map(|(version, _after)| version)
            .check("the tree-sitter-language-pack pin must close its quoted version")?;

        assert_eq!(
            PACK_VERSION, version,
            "PACK_VERSION and the tree-sitter-language-pack pin in Cargo.toml must move together"
        );
        Ok(())
    }
}
