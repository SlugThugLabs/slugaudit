use tree_sitter_language_pack::{detect_language_from_path, has_language};

#[must_use]
/// Maps a file path to the language its extension indicates, or `None` when
/// the pack does not name it.
pub fn detect_language(path: &str) -> Option<String> {
    detect_language_from_path(path).map(str::to_owned)
}

#[must_use]
/// True when the language pack can supply a parser for `language` on this
/// host right now.
pub fn language_available(language: &str) -> bool {
    has_language(language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_pack_language_by_path() {
        assert_eq!(detect_language("src/main.rs").as_deref(), Some("rust"));
    }
}
