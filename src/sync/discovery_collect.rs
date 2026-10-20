//! Routing of individual walker entries into discovered files or skips.
//!
//! Split from `discovery.rs` to keep that module under the small-file
//! budget. Two walker failures make the walked set untruthful and abort
//! discovery rather than become a per-file skip:
//!
//! - a yielded `ignore::Error` means a directory (or the project root)
//!   could not be listed, so no path under it is known;
//! - `DirEntry::error()` means an ignore file could not be applied and its
//!   rules were dropped, so files it meant to exclude may appear in the
//!   walked set.
//!
//! Either one would let a publish diff treat stored rows under the affected
//! subtree as deleted; discovery fails closed instead.

use super::discovery::{DiscoveredFile, DiscoveryError, SkippedFile, sniff_kind};
use crate::ignore_rules::is_excluded_path;
use std::path::Path;

/// True when a project-relative path is excluded by the hardcoded rules
/// shared with the watcher and incremental reconcile.
fn is_excluded(relative: &Path) -> bool {
    is_excluded_path(&relative.to_string_lossy())
}

/// The walked path's project-relative form, or the skip to report when the
/// walker yielded a path outside the root. `strip_prefix` is lexical, so
/// this should always succeed for the crate's own walker; a walker (or
/// future walker) that normalizes paths differently is surfaced in
/// `skipped` instead of being dropped silently.
pub(super) fn strip_project_prefix<'a>(
    root: &Path,
    absolute_path: &'a Path,
) -> Result<&'a Path, SkippedFile> {
    absolute_path.strip_prefix(root).map_err(|_| SkippedFile {
        reason: DiscoveryError::NotRelative(absolute_path.to_path_buf()).to_string(),
        absolute_path: absolute_path.to_path_buf(),
    })
}

/// Routes one walker entry into `discovered` or `skipped`, preserving the
/// walk's per-file skip reasons. A walker error or an entry carrying an
/// ignore-file error aborts discovery: those are the two cases where the
/// walked set is not the tree's file set, and continuing would let a
/// publish delete rows discovery simply could not see.
pub(super) fn collect_walk_entry(
    root: &Path,
    entry: Result<ignore::DirEntry, ignore::Error>,
    discovered: &mut Vec<DiscoveredFile>,
    skipped: &mut Vec<SkippedFile>,
) -> Result<(), DiscoveryError> {
    let entry = entry.map_err(DiscoveryError::Walk)?;
    if let Some(error) = entry.error() {
        return Err(DiscoveryError::IgnoreRules {
            kind: crate::watch::errors::ignore_error_kind(error),
            path: crate::watch::errors::ignore_error_path(error).map(Path::to_path_buf),
        });
    }
    if entry.file_type().is_some_and(|kind| kind.is_file()) {
        collect_file_entry(root, entry.path(), discovered, skipped);
    }
    Ok(())
}

/// Applies the include rules to one regular file and records it, or the
/// reason it was skipped.
fn collect_file_entry(
    root: &Path,
    entry_path: &Path,
    discovered: &mut Vec<DiscoveredFile>,
    skipped: &mut Vec<SkippedFile>,
) {
    let absolute_path = entry_path.to_path_buf();
    let relative_path = match strip_project_prefix(root, &absolute_path) {
        Ok(relative_path) => relative_path,
        Err(skipped_file) => {
            skipped.push(skipped_file);
            return;
        }
    };
    if is_excluded(relative_path) {
        return;
    }
    let kind = match sniff_kind(&absolute_path) {
        Ok(kind) => kind,
        Err(error) => {
            skipped.push(SkippedFile {
                reason: error.to_string(),
                absolute_path,
            });
            return;
        }
    };
    match relative_path.to_str() {
        Some(relative_path) => discovered.push(DiscoveredFile {
            relative_path: relative_path.replace('\\', "/"),
            absolute_path,
            kind,
        }),
        None => skipped.push(SkippedFile {
            reason: DiscoveryError::NonUtf8Path(absolute_path.clone()).to_string(),
            absolute_path,
        }),
    }
}
