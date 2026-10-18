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

    /// Must equal `TSLP_LANGUAGES` in `.cargo/config.toml`: the grammars compiled
    /// into the binary. Every other language is fetched by the pack at runtime.
    const BUNDLED: &[&str] = &[
        "c",
        "cpp",
        "csharp",
        "dart",
        "elixir",
        "go",
        "haskell",
        "java",
        "javascript",
        "json",
        "julia",
        "kotlin",
        "ocaml",
        "perl",
        "php",
        "python",
        "ruby",
        "rust",
        "swift",
        "tsx",
        "typescript",
    ];

    /// Regular files under `dir`, recursively.
    fn list_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .expect("list cache dir")
            .flat_map(|entry| {
                let path = entry.expect("cache entry").path();
                if path.is_dir() {
                    list_files(&path)
                } else {
                    vec![path]
                }
            })
            .collect()
    }

    /// Runs `body` with HOME and XDG_CACHE_HOME pointing at an empty directory, so
    /// the pack's download cache starts empty and any cache write is visible.
    fn with_empty_cache<T>(body: impl FnOnce() -> T) -> (T, Vec<std::path::PathBuf>) {
        let cache_home = tempfile::tempdir().expect("cache home");
        let _env = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
        let result = temp_env::with_vars(
            [
                ("HOME", Some(cache_home.path().as_os_str())),
                ("XDG_CACHE_HOME", Some(cache_home.path().as_os_str())),
            ],
            body,
        );
        // Other tests in the process call `has_language`, which makes the pack
        // register (and create) its cache directory under whatever HOME is
        // current, so empty directories are expected noise. A download or a
        // cache write leaves regular files, and only those count.
        let entries = list_files(cache_home.path());
        (result, entries)
    }

    #[test]
    fn detects_a_pack_language_by_path() {
        assert_eq!(detect_language("src/main.rs").as_deref(), Some("rust"));
    }

    #[test]
    fn the_bundled_list_equals_the_tslp_languages_build_setting() {
        let config = include_str!("../../.cargo/config.toml");
        let line = config
            .lines()
            .find(|line| line.starts_with("TSLP_LANGUAGES"))
            .expect("TSLP_LANGUAGES is set in .cargo/config.toml");
        let value = line
            .split('"')
            .nth(1)
            .expect("TSLP_LANGUAGES value is quoted");
        let mut configured: Vec<&str> = value.split(',').collect();
        configured.sort_unstable();

        assert_eq!(configured, BUNDLED);
    }

    #[test]
    fn every_bundled_language_parses_with_an_empty_cache_without_downloading() {
        let started = std::time::Instant::now();

        let (parsed, cache_entries) = with_empty_cache(|| {
            BUNDLED
                .iter()
                .map(|language| {
                    let mut parser = tree_sitter_language_pack::get_parser(language)
                        .unwrap_or_else(|error| panic!("{language} has no parser: {error}"));
                    (*language, parser.parse("x").is_some())
                })
                .collect::<Vec<_>>()
        });

        for (language, parsed) in parsed {
            assert!(parsed, "{language} produced no tree");
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "a download attempt would not return this fast"
        );
        assert!(
            cache_entries.is_empty(),
            "files were written to the download cache: {cache_entries:?}"
        );
    }

    #[test]
    fn a_language_outside_the_bundle_is_known_but_not_statically_available() {
        let ((known, usable), _) = with_empty_cache(|| {
            (
                tree_sitter_language_pack::has_language("zig"),
                tree_sitter_language_pack::has_parser("zig"),
            )
        });

        assert!(known, "zig must stay a known, downloadable language");
        assert!(!usable, "zig must not be compiled in or already cached");
    }

    #[test]
    #[ignore = "needs network: exercises the pack's real runtime download for a non-bundled language"]
    fn a_language_outside_the_bundle_is_downloaded_on_first_use() {
        let (loaded, cache_entries) =
            with_empty_cache(|| tree_sitter_language_pack::get_language("zig"));

        loaded.expect("zig downloads and loads");
        assert!(
            !cache_entries.is_empty(),
            "the download cache was not populated"
        );
    }
}
