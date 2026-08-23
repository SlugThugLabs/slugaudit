//! The resolution outcome model shared by every resolver: what an import
//! resolves to, and the candidate-picking that decides it.
//!
//! Split out of `generic.rs` so the data model is usable and testable on
//! its own, separate from the `LanguageResolver` trait and the
//! `GenericResolver` implementation.

use std::collections::HashSet;

/// The outcome of resolving an import reference.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub kind: ResolutionKind,
    pub confidence: Option<&'static str>,
    pub to_relative_path: Option<String>,
}

/// Whether an import was resolved to a real file, identified as external,
/// or left unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    Resolved,
    Unresolved,
    External,
}

impl ResolutionKind {
    pub fn as_sql_text(self) -> &'static str {
        match self {
            Self::Resolved => "Resolved",
            Self::Unresolved => "Unresolved",
            Self::External => "External",
        }
    }
}

/// Returns a resolution indicating the reference could not be resolved to
/// a project file.
pub fn unresolved() -> Resolution {
    Resolution {
        kind: ResolutionKind::Unresolved,
        confidence: None,
        to_relative_path: None,
    }
}

/// Returns a resolution indicating the reference is syntactically
/// identified as outside the project (standard library, third-party crate,
/// bare package name).
pub fn external() -> Resolution {
    Resolution {
        kind: ResolutionKind::External,
        confidence: None,
        to_relative_path: None,
    }
}

/// Picks the winning candidate out of a set of paths that would all
/// satisfy the reference: exactly one real match is `"High"` confidence,
/// more than one is `"Low"` (genuinely ambiguous — we still report the
/// first as a best guess rather than discarding the information).
pub fn pick(candidates: &[String], known_paths: &HashSet<&str>) -> Resolution {
    let matches: Vec<&String> = candidates
        .iter()
        .filter(|candidate| known_paths.contains(candidate.as_str()))
        .collect();
    match matches.len() {
        0 => unresolved(),
        1 => Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some("High"),
            to_relative_path: Some(matches[0].clone()),
        },
        _ => Resolution {
            kind: ResolutionKind::Resolved,
            confidence: Some("Low"),
            to_relative_path: Some(matches[0].clone()),
        },
    }
}
