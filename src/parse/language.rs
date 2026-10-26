use tree_sitter_language_pack::{detect_language_from_path, has_language};

#[must_use]
/// Maps a file path to the language its extension indicates, or `None` when
/// the pack does not name it.
pub fn detect_language(path: &str) -> Option<String> {
    detect_language_from_path(path).map(str::to_owned)
}

#[must_use]
/// True when the language pack knows `language` (`has_language`), whether or
/// not its grammar is on this host yet.
///
/// This is deliberately not `has_parser`, which is true only for a grammar
/// already downloaded and loadable: the pack fetches a grammar lazily from
/// its GitHub release the first time extraction needs it, so gating
/// extraction on `has_parser` would mean a grammar is never fetched. A
/// failed fetch surfaces from extraction as a typed pack error, which sync
/// records as `LoadFailed`.
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
