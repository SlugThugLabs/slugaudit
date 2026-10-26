use tree_sitter_language_pack::{detect_language_from_path, has_language};

/// The stored language of a prose file (Markdown, reStructuredText, AsciiDoc,
/// Org, plain text). It is not a grammar name: no grammar is loaded or run for
/// it, so `files.language = 'prose'` with `parse_outcome = 'NotAttempted'` and
/// `extraction_completeness = 'ContentOnly'` means "prose, deliberately not
/// parsed", unlike a pack language whose grammar failed to load (`LoadFailed`)
/// or a file with no detected language (`language IS NULL`).
pub const PROSE_LANGUAGE: &str = "prose";

/// Extensions (lowercase, no dot) that are prose rather than source or data.
const PROSE_EXTENSIONS: [&str; 9] = [
    "md", "markdown", "mdx", "rst", "adoc", "asciidoc", "org", "txt", "text",
];

#[must_use]
/// Maps a file path to the language its extension indicates, or `None` when
/// the pack does not name it. Prose files map to [`PROSE_LANGUAGE`], whatever
/// grammar the pack would assign (it maps `.txt` to `vimdoc`). Extensionless
/// names such as `README` get no language from the pack today and stay so.
pub fn detect_language(path: &str) -> Option<String> {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase);
    if extension.is_some_and(|ext| PROSE_EXTENSIONS.contains(&ext.as_str())) {
        return Some(PROSE_LANGUAGE.to_owned());
    }
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
    fn prose_extensions_classify_as_prose_and_data_or_source_stay_parsed() {
        let table = [
            ("docs/a.md", "prose"),
            ("a.markdown", "prose"),
            ("a.mdx", "prose"),
            ("a.rst", "prose"),
            ("a.adoc", "prose"),
            ("a.org", "prose"),
            ("notes.txt", "prose"),
            ("NOTES.TXT", "prose"),
            ("a.text", "prose"),
            ("data.json", "json"),
            ("ci.yaml", "yaml"),
            ("Cargo.toml", "toml"),
            ("main.rs", "rust"),
        ];
        for (path, expected) in table {
            assert_eq!(detect_language(path).as_deref(), Some(expected), "{path}");
        }
    }

    #[test]
    fn detects_a_pack_language_by_path() {
        assert_eq!(detect_language("src/main.rs").as_deref(), Some("rust"));
    }
}
