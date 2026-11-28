// slugaudit-line-exception: approved-by=split-judge; reason=shared resolution types, path normalization, language resolver registry, and multi-target deduplication
//! Language-specific and generic import resolution: shared outcome types,
//! project-relative path normalization, `OnceLock` resolver registry, and
//! deduplicated multi-target dispatch.

mod generic;

#[cfg(test)]
mod proptest;

#[cfg(test)]
mod gate_tests;

use crate::graph::WorkspaceCrates;
use crate::graph::resolve_rust::RustResolver;
use crate::graph::rust_mod_parents::RustModuleParents;
pub(crate) use generic::LanguageResolver;
use generic::{GenericResolver, GenericResolverConfig};
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::sync::OnceLock;

/// Hasher-erased membership lookup over known project-relative file paths so
/// `dyn LanguageResolver` works with any caller `HashSet<&str, S>`.
pub(crate) trait KnownPaths {
    fn contains_path(&self, path: &str) -> bool;
}

impl<S: BuildHasher> KnownPaths for HashSet<&str, S> {
    fn contains_path(&self, path: &str) -> bool {
        self.contains(path)
    }
}

/// The outcome of resolving an import reference.
#[derive(Debug, Clone)]
pub(crate) struct Resolution {
    pub kind: ResolutionKind,
    pub confidence: Option<&'static str>,
    pub to_relative_path: Option<String>,
}

/// Whether an import was resolved to a real file, identified as external,
/// or left unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ResolutionKind {
    Resolved,
    Unresolved,
    External,
}

impl ResolutionKind {
    pub(crate) fn as_sql_text(self) -> &'static str {
        match self {
            Self::Resolved => "Resolved",
            Self::Unresolved => "Unresolved",
            Self::External => "External",
        }
    }
}

/// Returns a resolution indicating the reference could not be resolved to
/// a project file.
pub(crate) fn unresolved() -> Resolution {
    Resolution {
        kind: ResolutionKind::Unresolved,
        confidence: None,
        to_relative_path: None,
    }
}

/// Returns a resolution indicating the reference is syntactically identified
/// as outside the project (standard library, third-party crate, bare package).
pub(crate) fn external() -> Resolution {
    Resolution {
        kind: ResolutionKind::External,
        confidence: None,
        to_relative_path: None,
    }
}

/// Picks the winning candidate out of a set of paths that would all satisfy the
/// reference: one match is `"High"` confidence; multiple matches are `"Low"`.
pub(crate) fn pick(candidates: &[String], known_paths: &dyn KnownPaths) -> Resolution {
    let matches: Vec<&String> = candidates
        .iter()
        .filter(|candidate| known_paths.contains_path(candidate.as_str()))
        .collect();
    match matches.as_slice() {
        [] => unresolved(),
        [only] => Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some("High"),
            to_relative_path: Some((*only).clone()),
        },
        [first, _, ..] => Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some("Low"),
            to_relative_path: Some((*first).clone()),
        },
    }
}

/// Joins path components with `/` and normalizes `..`/`.` segments, returning
/// `None` when a `..` climbs above the project root.
pub(crate) fn normalize_join(base_dir: &str, relative: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for part in base_dir.split('/').chain(relative.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

/// Returns the parent directory of a project-relative forward-slash path,
/// or the empty string for a root-level or empty path.
pub(crate) fn parent_dir(relative_path: &str) -> String {
    match relative_path.trim_end_matches('/').rsplit_once('/') {
        Some((parent, _)) => parent.to_owned(),
        None => String::new(),
    }
}

/// One deduplicated resolution outcome for a raw import statement.
pub(crate) struct ExtractedResolution {
    pub(crate) resolution: Resolution,
    pub(crate) syntax_unmodeled: bool,
}

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

/// Returns the resolver for `language`, falling back to `GenericResolver`.
pub(crate) fn get_resolver(language: &str) -> &'static dyn LanguageResolver {
    registry().get(language)
}

/// Resolves a single raw import source.
#[cfg(test)]
pub(crate) fn resolve_one(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    known_paths: &dyn KnownPaths,
) -> Resolution {
    let resolver = get_resolver(language);
    let Some(reference) = resolver.extract_reference(raw) else {
        return unresolved();
    };
    resolver.resolve(&reference, importing_relative_path, known_paths)
}

/// Resolves every reference extracted from one raw import statement within
/// optional enclosing inline Rust `mod` segments, deduplicating identical
/// targets and identical targetless verdicts in order.
pub(crate) fn resolve_many_scoped(
    language: &str,
    importing_relative_path: &str,
    raw: &str,
    inline_segments: &[String],
    known_paths: &dyn KnownPaths,
    rust_parents: &RustModuleParents,
    workspace_crates: &WorkspaceCrates,
) -> Vec<ExtractedResolution> {
    let resolver = get_resolver(language);
    let is_rust = language == "rust";
    let references = if is_rust {
        match RustResolver::parse_references(raw) {
            Some(refs) if refs.is_empty() => return Vec::new(),
            Some(refs) => refs,
            None => {
                return vec![ExtractedResolution {
                    resolution: unresolved(),
                    syntax_unmodeled: true,
                }];
            }
        }
    } else {
        let refs = resolver.extract_references(raw);
        if refs.is_empty() {
            if resolver.is_modeled_syntax(raw) {
                return Vec::new();
            }
            return vec![ExtractedResolution {
                resolution: unresolved(),
                syntax_unmodeled: true,
            }];
        }
        refs
    };

    let mut dedup = DedupState::new();
    for reference in &references {
        let resolution = if is_rust {
            RustResolver::resolve_in_inline_scope(
                reference,
                importing_relative_path,
                inline_segments,
                known_paths,
                rust_parents,
                workspace_crates,
            )
        } else {
            resolver.resolve_with_rust_context(
                reference,
                importing_relative_path,
                known_paths,
                rust_parents,
                workspace_crates,
            )
        };
        dedup.push(resolution, false);
    }
    dedup.results
}

fn confidence_rank(confidence: Option<&'static str>) -> u8 {
    match confidence {
        Some("High") => 2,
        Some("Low") => 1,
        _ => 0,
    }
}

struct DedupState {
    results: Vec<ExtractedResolution>,
    by_target: HashMap<(ResolutionKind, String), usize>,
    targetless: HashSet<(ResolutionKind, Option<&'static str>, bool)>,
}

impl DedupState {
    fn new() -> Self {
        Self {
            results: Vec::new(),
            by_target: HashMap::new(),
            targetless: HashSet::new(),
        }
    }

    fn push(&mut self, resolution: Resolution, syntax_unmodeled: bool) {
        if let Some(target_path) = resolution.to_relative_path.as_deref() {
            let key = (resolution.kind, target_path.to_owned());
            if let Some(&idx) = self.by_target.get(&key) {
                if let Some(existing) = self.results.get_mut(idx)
                    && confidence_rank(resolution.confidence)
                        > confidence_rank(existing.resolution.confidence)
                {
                    existing.resolution.confidence = resolution.confidence;
                }
                return;
            }
            let idx = self.results.len();
            self.by_target.insert(key, idx);
        } else if !self.targetless.insert((
            resolution.kind,
            resolution.confidence,
            syntax_unmodeled,
        )) {
            return;
        }

        self.results.push(ExtractedResolution {
            resolution,
            syntax_unmodeled,
        });
    }
}
