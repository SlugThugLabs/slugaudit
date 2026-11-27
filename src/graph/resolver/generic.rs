// slugaudit-line-exception: approved-by=split-judge; reason=LanguageResolver trait and consolidated GenericResolver (JS/TS, Python, and generic import extraction and candidate path resolution)
//! The `LanguageResolver` trait, `GenericResolverConfig`, and the consolidated
//! `GenericResolver` implementation (JS/TS, Python, and multi-language
//! import extraction and candidate path resolution).

use super::{
    KnownPaths, Resolution, ResolutionKind, external, normalize_join, parent_dir, pick, unresolved,
};
use crate::graph::rust_mod_parents::RustModuleParents;
use crate::graph::{ImportReference, WorkspaceCrates};

/// A language-specific import resolver.
pub(crate) trait LanguageResolver: Send + Sync {
    /// Returns true if this resolver handles the given language name.
    fn supports(&self, language: &str) -> bool;

    /// Extract the module/path reference from raw import statement text.
    fn extract_reference(&self, raw: &str) -> Option<ImportReference>;

    /// Extract all module/path references from raw import statement text.
    fn extract_references(&self, raw: &str) -> Vec<ImportReference> {
        self.extract_reference(raw).into_iter().collect()
    }

    /// Returns `true` when `raw` matches modeled import syntax for this
    /// language, even if it yields zero targets (e.g. `use foo::{};`).
    fn is_modeled_syntax(&self, raw: &str) -> bool {
        !self.extract_references(raw).is_empty()
    }

    /// Resolve a reference to a project file.
    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &dyn KnownPaths,
    ) -> Resolution;

    /// Resolve a reference with optional Rust `#[path]` parent metadata.
    #[cfg(test)]
    fn resolve_with_rust_parents(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &dyn KnownPaths,
        rust_parents: &RustModuleParents,
    ) -> Resolution {
        self.resolve_with_rust_context(
            reference,
            importing_relative_path,
            known_paths,
            rust_parents,
            &WorkspaceCrates::new(),
        )
    }

    /// Resolve a reference with optional Rust `#[path]` parent metadata and
    /// Cargo workspace sibling crate roots.
    fn resolve_with_rust_context(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &dyn KnownPaths,
        _rust_parents: &RustModuleParents,
        _workspace_crates: &WorkspaceCrates,
    ) -> Resolution {
        self.resolve(reference, importing_relative_path, known_paths)
    }
}

/// Configuration for the generic import resolver.
#[derive(Debug, Clone)]
pub(crate) struct GenericResolverConfig {
    pub extensions: Vec<&'static str>,
    pub index_filename: Option<&'static str>,
    pub module_separator: &'static str,
    pub relative_prefixes: Vec<&'static str>,
    pub bare_names_are_external: bool,
}

impl Default for GenericResolverConfig {
    fn default() -> Self {
        Self {
            extensions: vec![
                "py", "js", "ts", "jsx", "tsx", "mjs", "cjs", "rb", "go", "rs", "java", "c", "cpp",
                "cc", "h", "hpp", "hh", "kt", "kts", "swift", "cs", "dart", "scala", "sc", "hs",
                "lua", "php", "pl", "pm", "ex", "exs", "ml", "sol", "jl", "erl", "hrl",
            ],
            index_filename: None,
            module_separator: ".",
            relative_prefixes: vec!["./", "../"],
            bare_names_are_external: true,
        }
    }
}

/// Generic import resolver handling relative paths, module paths, and bare
/// package names across Python, JS/TS, and catch-all languages.
pub(crate) struct GenericResolver {
    config: GenericResolverConfig,
    languages: Vec<&'static str>,
}

impl GenericResolver {
    pub(crate) fn new(config: GenericResolverConfig) -> Self {
        Self {
            config,
            languages: Vec::new(),
        }
    }

    pub(crate) fn python() -> Self {
        Self {
            config: GenericResolverConfig {
                extensions: vec!["py"],
                index_filename: Some("__init__"),
                module_separator: ".",
                relative_prefixes: vec![],
                bare_names_are_external: true,
            },
            languages: vec!["python", "python3"],
        }
    }

    pub(crate) fn js() -> Self {
        Self {
            config: GenericResolverConfig {
                extensions: vec!["js", "ts", "jsx", "tsx", "mjs", "cjs"],
                index_filename: Some("index"),
                module_separator: "/",
                relative_prefixes: vec!["./", "../"],
                bare_names_are_external: true,
            },
            languages: vec!["javascript", "typescript", "jsx", "tsx"],
        }
    }
}

impl LanguageResolver for GenericResolver {
    fn supports(&self, language: &str) -> bool {
        self.languages.is_empty() || self.languages.contains(&language)
    }

    fn extract_reference(&self, raw: &str) -> Option<ImportReference> {
        extract_reference(&self.config, raw)
    }

    fn resolve(
        &self,
        reference: &ImportReference,
        importing_relative_path: &str,
        known_paths: &dyn KnownPaths,
    ) -> Resolution {
        let text = reference.text.trim();

        if self
            .config
            .relative_prefixes
            .iter()
            .any(|prefix| text.starts_with(prefix))
        {
            let base_dir = parent_dir(importing_relative_path);
            return resolve_relative_path(&base_dir, text, &self.config, known_paths);
        }

        if starts_with_python_dot_prefix(text) {
            return resolve_python_relative(
                text,
                &self.config,
                importing_relative_path,
                known_paths,
            );
        }

        if text.starts_with("package:") || text.starts_with("dart:") {
            return external();
        }

        if !text.contains(self.config.module_separator) {
            return external_or_unresolved(&self.config);
        }

        let fs_path = module_path_to_fs_path(text, &self.config);
        candidate_paths(&fs_path, &self.config, known_paths)
    }
}

fn extract_reference(config: &GenericResolverConfig, raw: &str) -> Option<ImportReference> {
    let trimmed = raw.trim();

    if trimmed.starts_with("./") || trimmed.starts_with("../") {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }

    if trimmed.starts_with("import") && trimmed.split_whitespace().any(|token| token == "from") {
        return extract_js_reference(trimmed);
    }

    if let Some(reference) = extract_python_reference(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_quoted_import(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_csharp_or_julia(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_perl_or_php(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_ocaml(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_include(trimmed) {
        return Some(reference);
    }

    extract_bare_package(config, trimmed)
}

fn extract_js_reference(raw: &str) -> Option<ImportReference> {
    let trimmed = raw.trim();
    if !trimmed.contains("from") {
        return None;
    }
    let text = extract_quoted_string(trimmed)?;
    Some(ImportReference { text })
}

fn extract_python_reference(trimmed: &str) -> Option<ImportReference> {
    if trimmed == "." {
        return Some(ImportReference {
            text: ".".to_owned(),
        });
    }
    if starts_with_python_dot_prefix(trimmed) {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }
    if let Some(after_from) = strip_keyword(trimmed, "from") {
        let module = after_from.split_whitespace().next()?;
        if !module.starts_with('"') && !module.starts_with('\'') {
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }
        return None;
    }
    if let Some(after_import) = strip_keyword(trimmed, "import") {
        let module = after_import.split_whitespace().next()?;
        if !module.starts_with('"') && !module.starts_with('\'') {
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }
        return None;
    }
    None
}

fn resolve_python_relative(
    text: &str,
    config: &GenericResolverConfig,
    importing_relative_path: &str,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    let dots = text.chars().take_while(|c| *c == '.').count();
    let Some(remainder) = text.get(dots..) else {
        return unresolved();
    };
    let Some(levels_up) = dots.checked_sub(1) else {
        return unresolved();
    };

    let mut base_dir = parent_dir(importing_relative_path);
    let depth = base_dir.split('/').filter(|part| !part.is_empty()).count();
    if levels_up > depth {
        return unresolved();
    }
    for _ in 0..levels_up {
        base_dir = parent_dir(&base_dir);
    }

    let joined = if remainder.is_empty() {
        normalize_join(&base_dir, "")
    } else {
        normalize_join(&base_dir, &remainder.replace('.', "/"))
    };
    let Some(path) = joined else {
        return unresolved();
    };

    if remainder.is_empty() {
        if let Some(index) = config.index_filename {
            let init_path = if path.is_empty() {
                format!("{index}.py")
            } else {
                format!("{path}/{index}.py")
            };
            if known_paths.contains_path(init_path.as_str()) {
                return Resolution {
                    kind: ResolutionKind::Resolved,
                    confidence: Some("High"),
                    to_relative_path: Some(init_path),
                };
            }
        }
        unresolved()
    } else {
        candidate_paths(&path, config, known_paths)
    }
}

fn extract_quoted_import(trimmed: &str) -> Option<ImportReference> {
    if !trimmed.starts_with("import") {
        return None;
    }
    let quoted = extract_quoted_string(trimmed)?;
    if quoted.starts_with("./")
        || quoted.starts_with("../")
        || quoted.starts_with("package:")
        || quoted.starts_with("dart:")
    {
        return Some(ImportReference { text: quoted });
    }
    None
}

fn extract_csharp_or_julia(trimmed: &str) -> Option<ImportReference> {
    let after_using = strip_keyword(trimmed, "using")?;
    let mut tokens = after_using.split_whitespace();
    let mut module = tokens.next()?.trim_end_matches(';');
    if module == "static" {
        module = tokens.next()?.trim_end_matches(';');
    }
    Some(ImportReference {
        text: module.to_owned(),
    })
}

fn extract_perl_or_php(trimmed: &str) -> Option<ImportReference> {
    let after_use = strip_keyword(trimmed, "use")?;
    let module = after_use.split_whitespace().next()?.trim_end_matches(';');
    let module = module.replace("::", ".").replace('\\', ".");
    Some(ImportReference { text: module })
}

fn extract_ocaml(trimmed: &str) -> Option<ImportReference> {
    let after_open = strip_keyword(trimmed, "open")?;
    let module = after_open.split_whitespace().next()?;
    Some(ImportReference {
        text: module.to_owned(),
    })
}

fn extract_include(trimmed: &str) -> Option<ImportReference> {
    let rest = trimmed.strip_prefix("#include")?;
    let inner = rest.trim();
    let path = inner
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('<').and_then(|r| r.strip_suffix('>')))?;
    Some(ImportReference {
        text: format!("./{}", path.trim()),
    })
}

fn extract_bare_package(config: &GenericResolverConfig, trimmed: &str) -> Option<ImportReference> {
    if config.bare_names_are_external
        && !trimmed.is_empty()
        && !trimmed.starts_with('.')
        && !trimmed.contains(config.module_separator)
        && !trimmed.contains(' ')
        && !trimmed.contains('"')
        && !trimmed.contains('\'')
    {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }
    None
}

fn extract_quoted_string(text: &str) -> Option<String> {
    let quote = text
        .chars()
        .find(|character| matches!(character, '\'' | '"'))?;
    let (_, after_open) = text.split_once(quote)?;
    let (literal, _) = after_open.split_once(quote)?;
    Some(literal.to_owned())
}

fn module_path_to_fs_path(text: &str, config: &GenericResolverConfig) -> String {
    if config.module_separator == "/" {
        text.to_owned()
    } else {
        text.replace(config.module_separator, "/")
    }
}

fn candidate_paths(
    path: &str,
    config: &GenericResolverConfig,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    let mut candidates = vec![path.to_owned()];
    for ext in &config.extensions {
        candidates.push(format!("{path}.{ext}"));
    }
    if let Some(index) = config.index_filename {
        for ext in &config.extensions {
            candidates.push(format!("{path}/{index}.{ext}"));
        }
        candidates.push(format!("{path}/{index}"));
    }
    pick(&candidates, known_paths)
}

fn resolve_relative_path(
    base_dir: &str,
    text: &str,
    config: &GenericResolverConfig,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    let Some(path) = normalize_join(base_dir, text) else {
        return unresolved();
    };
    candidate_paths(&path, config, known_paths)
}

fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let after = text.strip_prefix(keyword)?;
    after
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then_some(after)
}

fn starts_with_python_dot_prefix(text: &str) -> bool {
    text.starts_with('.') && !text.starts_with("./") && !text.starts_with("../")
}

fn external_or_unresolved(config: &GenericResolverConfig) -> Resolution {
    if config.bare_names_are_external {
        external()
    } else {
        unresolved()
    }
}
