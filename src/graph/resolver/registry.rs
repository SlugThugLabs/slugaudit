//! The resolver registry and the public entry points used by
//! [`crate::graph::mod.rs`].
//!
//! Keeps registry state, the `OnceLock`-initialized lookup table, and the
//! cross-language dispatch (e.g. `resolve_one` looking up the right resolver
//! for an import's language and invoking it) in one file. The registry's
//! initialization is the one place that knows about *every* registered
//! resolver: if a new language ships, this is the only file that
//! needs to grow.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::generic::{GenericResolver, GenericResolverConfig, LanguageResolver};
use super::types::Resolution;
use crate::graph::resolve_rust::RustResolver;
use crate::graph::rust_mod_parents::RustModuleParents;

/// One deduplicated resolution outcome for a raw import statement.
pub(crate) struct ExtractedResolution {
    pub(crate) resolution: Resolution,
    pub(crate) syntax_unmodeled: bool,
}

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
        if let Some(resolver) = self
            .specific_resolvers
            .iter()
            .find(|r| r.supports(language))
        {
            return resolver.as_ref();
        }
        &self.fallback_resolver
    }
}

static REGISTRY: OnceLock<ResolverRegistry> = OnceLock::new();

fn registry() -> &'static ResolverRegistry {
    REGISTRY.get_or_init(|| {
        let mut reg = ResolverRegistry::new();
        reg.register(Box::new(RustResolver));
        reg.register(Box::new(GenericResolver::python()));
        reg.register(Box::new(GenericResolver::js()));
        reg
    })
}

/// Returns the resolver for `language`. Falls back to the generic
/// resolver if no specific resolver supports the language.
pub(crate) fn get_resolver(language: &str) -> &'static dyn LanguageResolver {
    registry().get(language)
}

/// Resolves a single raw import source. Returns a `Resolution` — never
/// panics or returns an error.
#[cfg(test)]
pub(crate) fn resolve_one(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    known_paths: &HashSet<&str>,
) -> Resolution {
    let resolver = get_resolver(language);

    let Some(reference) = resolver.extract_reference(raw) else {
        return super::types::unresolved();
    };

    resolver.resolve(&reference, importing_relative_path, known_paths)
}

/// Resolves every reference extracted from one raw import statement,
/// deduplicating identical targets and identical targetless verdicts in
/// source order.
pub(crate) fn resolve_many(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    known_paths: &HashSet<&str>,
    rust_parents: &RustModuleParents,
) -> Vec<ExtractedResolution> {
    let resolver = get_resolver(language);
    let references = resolver.extract_references(raw);
    if references.is_empty() {
        if resolver.is_modeled_syntax(raw) {
            return Vec::new();
        }
        return vec![ExtractedResolution {
            resolution: super::types::unresolved(),
            syntax_unmodeled: true,
        }];
    }

    let mut results: Vec<ExtractedResolution> = Vec::new();
    for reference in &references {
        let resolution = resolver.resolve_with_rust_parents(
            reference,
            importing_relative_path,
            known_paths,
            rust_parents,
        );
        push_deduped(&mut results, resolution, false);
    }
    results
}

fn confidence_rank(confidence: Option<&'static str>) -> u8 {
    match confidence {
        Some("High") => 3,
        Some("Medium") => 2,
        Some("Low") => 1,
        _ => 0,
    }
}

fn push_deduped(
    results: &mut Vec<ExtractedResolution>,
    resolution: Resolution,
    syntax_unmodeled: bool,
) {
    if let Some(target_path) = resolution.to_relative_path.as_deref() {
        if let Some(existing) = results.iter_mut().find(|entry| {
            entry.resolution.kind == resolution.kind
                && entry.resolution.to_relative_path.as_deref() == Some(target_path)
        }) {
            if confidence_rank(resolution.confidence)
                > confidence_rank(existing.resolution.confidence)
            {
                existing.resolution.confidence = resolution.confidence;
            }
            return;
        }
    } else if results.iter().any(|entry| {
        entry.resolution.to_relative_path.is_none()
            && entry.resolution.kind == resolution.kind
            && entry.resolution.confidence == resolution.confidence
            && entry.syntax_unmodeled == syntax_unmodeled
    }) {
        return;
    }

    results.push(ExtractedResolution {
        resolution,
        syntax_unmodeled,
    });
}
