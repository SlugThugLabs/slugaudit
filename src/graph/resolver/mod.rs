//! Language-specific and generic import resolution.
//!
//! Split into submodules so each file stays under the small-file-rule
//! cap. The split files are:
//!
//! - [`types`] — the outcome data model (`Resolution` / `ResolutionKind` /
//!   `pick` / `unresolved` / `external`), usable and testable on its own.
//! - [`extract`] — the raw import-statement parser
//!   (`GenericResolver::extract_reference` as a free function).
//! - [`generic`] — the `LanguageResolver` trait, `GenericResolverConfig`,
//!   and the `GenericResolver` struct (dispatcher + resolution step).
//! - [`python`] — Python-style relative imports (`from . import ...`)
//!   and the `__init__.py` index-file handling.
//! - [`js`] — JS/TS-style `import ... from 'path'` reference extraction.
//! - [`path_helpers`] — path-arithmetic helpers used across both the
//!   generic resolver and the language-specific ones
//!   (`extract_quoted_string`, `module_path_to_fs_path`,
//!   `candidate_paths`, `resolve_relative_path`).
//! - [`registry`] — the `OnceLock`-backed resolver lookup and the
//!   public entry points (`get_resolver`, `is_supported_language`,
//!   `resolve_one`).
//!
//! Public API re-exported below so callers continue to use
//! `crate::graph::resolver::Resolution` etc. without caring which
//! submodule defines them.

mod extract;
mod generic;
mod js;
mod path_helpers;
mod python;
mod registry;
mod types;

#[cfg(test)]
#[path = "proptest.rs"]
mod proptest;

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;

// Re-exports that comprise the resolver's public API surface. These
// are used by callers of this module, not directly inside `mod.rs`,
// so the unused-import lint would otherwise flag every name here.
pub(crate) use generic::LanguageResolver;
pub(crate) use registry::{get_resolver, is_supported_language, resolve_one};
pub(crate) use types::{Resolution, ResolutionKind, external, pick, unresolved};
