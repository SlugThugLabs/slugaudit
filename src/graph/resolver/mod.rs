//! Language-specific and generic import resolution.
//!
//! Split into submodules so each file stays under the small-file-rule
//! cap. The split files are:
//!
//! - [`generic`] — `Resolution` / `ResolutionKind` / `LanguageResolver`
//!   trait / `GenericResolver` struct. Caller-facing types and the
//!   dispatcher logic.
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

mod generic;
mod js;
mod path_helpers;
mod python;
mod registry;

#[cfg(test)]
#[path = "proptest.rs"]
mod proptest;

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;

// Re-exports that comprise the resolver's public API surface. These
// are used by callers of this module, not directly inside `mod.rs`,
// so the unused-import lint would otherwise flag every name here.
#[allow(unused_imports)]
pub use generic::{
    GenericResolver, GenericResolverConfig, LanguageResolver, Resolution, ResolutionKind, external,
    pick, unresolved,
};
#[allow(unused_imports)]
pub use registry::{get_resolver, is_supported_language, resolve_one};
