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

/// File names (exact, no directory) of ignore-pattern lists: plain path lists
/// with no grammar, stored like prose (the pack names no language for them, so
/// they would otherwise be `language IS NULL` and read as a parser failure).
/// The class is the same "stored, searchable, deliberately not parsed".
const IGNORE_FILE_NAMES: [&str; 3] = [".gitignore", ".ignore", ".dockerignore"];

#[must_use]
/// True when `name` is the stored prose language or a pack grammar name that
/// the prose table shadows (the pack names `.md` `markdown`, `.txt`
/// `vimdoc`, ...). Derived from [`PROSE_EXTENSIONS`], so there is no second
/// list: a request for such a language can never match an indexed file
/// because [`detect_language`] stores those files as [`PROSE_LANGUAGE`].
pub fn is_prose_language_name(name: &str) -> bool {
    name == PROSE_LANGUAGE
        || PROSE_EXTENSIONS.iter().any(|extension| {
            detect_language_from_path(&format!("x.{extension}")).is_some_and(|pack| pack == name)
        })
}

#[must_use]
/// Maps a file path to the language its extension indicates, or `None` when
/// the pack does not name it. Prose files map to [`PROSE_LANGUAGE`], whatever
/// grammar the pack would assign (it maps `.txt` to `vimdoc`), and so do the
/// ignore-pattern lists named by [`IGNORE_FILE_NAMES`] (`.gitignore`, ...).
/// Other extensionless names such as `README` get no language from the pack
/// today and stay so.
pub fn detect_language(path: &str) -> Option<String> {
    let is_ignore_file = std::path::Path::new(path)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| IGNORE_FILE_NAMES.contains(&name));
    if is_ignore_file {
        return Some(PROSE_LANGUAGE.to_owned());
    }
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
    fn ignore_pattern_lists_are_stored_as_prose_at_any_depth() {
        for path in [".gitignore", "sub/.gitignore", ".ignore", ".dockerignore"] {
            assert_eq!(detect_language(path).as_deref(), Some("prose"), "{path}");
        }
        for path in ["gitignore", ".gitignore.bak", ".npmignore"] {
            assert_eq!(detect_language(path), None, "{path}");
        }
    }

    #[test]
    fn prose_language_names_include_the_shadowed_pack_grammars() {
        for name in ["prose", "markdown", "rst", "org", "vimdoc"] {
            assert!(is_prose_language_name(name), "{name}");
        }
        for name in ["rust", "json", "yaml", "toml", "python"] {
            assert!(!is_prose_language_name(name), "{name}");
        }
    }

    #[test]
    fn detects_a_pack_language_by_path() {
        assert_eq!(detect_language("src/main.rs").as_deref(), Some("rust"));
    }
}
