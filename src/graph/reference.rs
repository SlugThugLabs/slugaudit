//! A module/path reference as written in source, stripped of its
//! surrounding statement syntax but not yet resolved to a file.

/// A module/path reference as written in source. `text` is the raw
/// module path (e.g. `"./utils"`, `"crate::foo::bar"`, `".bar"`)
/// extracted from the import statement by a `LanguageResolver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReference {
    pub text: String,
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
