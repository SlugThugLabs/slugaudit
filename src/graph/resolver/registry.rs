//! The resolver registry and the public entry points used by
//! [`crate::graph::mod.rs`].
//!
//! Extracted from `src/graph/resolver.rs` so registry state, the
//! `OnceLock`-initialized lookup table, and the cross-language dispatch
//! (e.g. `resolve_one` looking up the right resolver for an import's
//! language and invoking it) live in one file. The registry's
//! initialization is the one place that knows about *every* registered
//! resolver: if a new language ships, this is the only file that
//! needs to grow.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::generic::{GenericResolver, GenericResolverConfig, LanguageResolver, Resolution};
use crate::graph::resolve_rust::RustResolver;

/// Registry of language-specific resolvers. Language-specific resolvers
/// (including configured `GenericResolver` instances for Python and JS)
/// are checked first; the catch-all generic resolver is the fallback
/// for any language without a specific resolver.
struct ResolverRegistry {
    specific_resolvers: Vec<Box<dyn LanguageResolver>>,
    fallback_resolver: GenericResolver,
}

impl ResolverRegistry {
    fn new() -> Self {
        Self {
            specific_resolvers: Vec::new(),
            fallback_resolver: GenericResolver::new(GenericResolverConfig::default()),
        }
    }

    fn register(&mut self, resolver: Box<dyn LanguageResolver>) {
        self.specific_resolvers.push(resolver);
    }

    fn get(&self, language: &str) -> &dyn LanguageResolver {
        // Try specific resolvers first (includes Python/JS generic
        // resolvers).
        if let Some(resolver) = self
            .specific_resolvers
            .iter()
            .find(|r| r.supports(language))
        {
            return resolver.as_ref();
        }
        // Fall back to catch-all generic resolver.
        &self.fallback_resolver
    }
}

static REGISTRY: OnceLock<ResolverRegistry> = OnceLock::new();

fn registry() -> &'static ResolverRegistry {
    REGISTRY.get_or_init(|| {
        let mut reg = ResolverRegistry::new();
        // Register language-specific resolvers. These take precedence
        // over the catch-all generic resolver.
        reg.register(Box::new(RustResolver));
        reg.register(Box::new(GenericResolver::python()));
        reg.register(Box::new(GenericResolver::js()));
        reg
    })
}

/// Returns the resolver for `language`. Falls back to the generic
/// resolver if no specific resolver supports the language.
pub fn get_resolver(language: &str) -> &'static dyn LanguageResolver {
    registry().get(language)
}

/// Returns true if a language-specific resolver (not the generic
/// fallback) supports `language`.
pub fn is_supported_language(language: &str) -> bool {
    registry()
        .specific_resolvers
        .iter()
        .any(|r| r.supports(language))
}

/// Resolves a single raw import source. Returns a `Resolution` — never
/// panics or returns an error.
pub fn resolve_one(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let resolver = get_resolver(language);

    let Some(reference) = resolver.extract_reference(raw) else {
        return super::generic::unresolved();
    };

    resolver.resolve(&reference, importing_relative_path, known_paths)
}
