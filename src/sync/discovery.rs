use crate::ignore_rules::{indexable_walker, is_excluded_path};
use crate::model::process_limits;
use crate::util::Deadline;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Bytes sampled from the start of a file to decide binary vs. text, same
/// heuristic class as git/ripgrep: a NUL byte in the sample means binary.
const BINARY_SNIFF_BYTES: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// What the walk concluded about a file before sampling: to be indexed,
/// binary (content not retained), or skipped with a reason.
pub enum FileKind {
    Indexed,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One file yielded by the discovery walk: relative and absolute paths, kind,
/// and the size/mtime signals the sample stage re-checks.
pub struct DiscoveredFile {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub kind: FileKind,
}

/// A file the walk found but could not include, with why. Discovery
/// continues past these rather than failing the entire project — one
/// unreadable or non-UTF8-named file must not make every file in the
/// project unqueryable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub absolute_path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Error)]
/// Walk-level failures. Unrepresentable or unreadable paths become reported
/// `skipped` entries; a failed root read aborts discovery with the reason.
pub enum DiscoveryError {
    #[error("failed to walk project files: {0}")]
    Walk(#[from] ignore::Error),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("discovered path {0} is not relative to the project root")]
    NotRelative(PathBuf),
    /// Relative paths are stored and queried as UTF-8 text. A non-UTF-8
    /// path is rejected rather than lossily rewritten, which would make
    /// the stored path unusable for round-tripping back to disk.
    #[error("discovered path {0} is not valid UTF-8")]
    NonUtf8Path(PathBuf),
    #[error("discovery exceeded its wall-clock time budget after {elapsed_ms} ms")]
    TimeBudgetExceeded { elapsed_ms: u128 },
}

impl DiscoveryError {
    /// The elapsed budget when this is a budget exhaustion, else `None`.
    /// A publish wraps this variant in [`super::publish::PublishError::Discovery`],
    /// so the classifier has to be reachable through the wrapper.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            DiscoveryError::TimeBudgetExceeded { elapsed_ms } => Some(*elapsed_ms),
            _ => None,
        }
    }
}

fn is_excluded(relative: &Path) -> bool {
    // The hardcoded exclusion set (SlugAudit's own data dir, VCS
    // internals, scratch files) lives in `crate::ignore_rules` and is
    // shared with the watcher and incremental reconcile, so all three
    // paths agree on what is indexable.
    is_excluded_path(&relative.to_string_lossy())
}

/// The project-relative path of a file the walk skipped, when that path can
/// be stored as text. `None` for a non-UTF-8 name, a path outside the root,
/// or the root itself (a walk error has no file behind it) — none of those
/// can be a `files.path` value, so such a file stays visible only in the
/// publish report's `skipped` list and the stderr log, never in the index.
#[must_use]
pub(super) fn representable_relative_path(root: &Path, absolute_path: &Path) -> Option<String> {
    let relative = absolute_path.strip_prefix(root).ok()?;
    let relative = relative.to_str()?;
    if relative.is_empty() {
        return None;
    }
    Some(relative.replace('\\', "/"))
}

/// The project-relative raw bytes of a discovered path that cannot be
/// represented as UTF-8 text, or `None` when the path is representable,
/// outside the root, or the root itself (a walk error has no file behind
/// it). These are exactly the files a `files.path` row cannot hold, so the
/// persisted set this feeds is the only place the index can count them.
#[must_use]
pub(super) fn unrepresentable_relative_bytes(root: &Path, absolute_path: &Path) -> Option<Vec<u8>> {
    let relative = absolute_path.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty() || relative.to_str().is_some() {
        return None;
    }
    Some(relative.as_os_str().as_encoded_bytes().to_vec())
}

/// The unrepresentable subset of a discovery walk's skipped files. Used by
/// the full publish to persist the set; the incremental stat sweep calls
/// [`unrepresentable_relative_bytes`] per walked entry instead.
#[must_use]
pub(super) fn unrepresentable_paths(root: &Path, skipped: &[SkippedFile]) -> HashSet<Vec<u8>> {
    skipped
        .iter()
        .filter_map(|file| unrepresentable_relative_bytes(root, &file.absolute_path))
        .collect()
}

/// Sniffs whether `path` is binary (a NUL byte in the first
/// `BINARY_SNIFF_BYTES`) or indexable text. This is the single source of
/// truth for binary-ness: the initial import calls it during discovery,
/// and incremental reconcile re-calls it for dirty paths so a file whose
/// binary-ness changed on disk is classified the same way it would be on a
/// fresh import.
pub(crate) fn sniff_kind(path: &Path) -> Result<FileKind, DiscoveryError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|source| DiscoveryError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buffer = [0_u8; BINARY_SNIFF_BYTES];
    let read = file
        .read(&mut buffer)
        .map_err(|source| DiscoveryError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if buffer[..read].contains(&0) {
        Ok(FileKind::Binary)
    } else {
        Ok(FileKind::Indexed)
    }
}

/// Walks the project root for non-binary and binary files alike, honoring
/// standard ignore files, never descending into VCS internals or
/// SlugAudit's own activation directory, and never following symlinks that
/// leave the project root. Returns discovered files in deterministic sorted
/// order, plus any files the walk found but could not include (unreadable,
/// non-UTF8 path, etc.) — those are skipped individually rather than
/// failing the whole walk, so a single bad file can't make an entire
/// project unqueryable.
///
/// # Errors
///
/// Returns an error only if the walk itself fails outright (e.g. the
/// project root isn't readable at all).
pub fn discover(root: &Path) -> Result<(Vec<DiscoveredFile>, Vec<SkippedFile>), DiscoveryError> {
    discover_with_deadline(root, &Deadline::after(process_limits().max_sync_wall_clock))
}

/// The walked path's project-relative form, or the skip to report when the
/// walker yielded a path outside the root. `strip_prefix` is lexical, so
/// this should always succeed for the crate's own walker; a walker (or
/// future walker) that normalizes paths differently is surfaced in
/// `skipped` instead of being dropped silently.
fn strip_project_prefix<'a>(root: &Path, absolute_path: &'a Path) -> Result<&'a Path, SkippedFile> {
    absolute_path.strip_prefix(root).map_err(|_| SkippedFile {
        reason: DiscoveryError::NotRelative(absolute_path.to_path_buf()).to_string(),
        absolute_path: absolute_path.to_path_buf(),
    })
}

/// [`discover`] under an explicit wall-clock [`Deadline`], checked once per
/// walked entry so a pathological tree (huge, or on a hung network
/// filesystem) fails closed with [`DiscoveryError::TimeBudgetExceeded`]
/// instead of stalling the publish forever. The deadline is created by the
/// caller so the whole operation — discovery plus sampling plus retries —
/// shares one budget.
pub(crate) fn discover_with_deadline(
    root: &Path,
    deadline: &Deadline,
) -> Result<(Vec<DiscoveredFile>, Vec<SkippedFile>), DiscoveryError> {
    // `standard_filters` sets several flags at once, including `hidden`; it
    // must run before the explicit `hidden(false)` override or it silently
    // wins and dotfiles/dotdirs (e.g. `.github/`) vanish from the walk.
    let walker = indexable_walker(root);

    let mut discovered = Vec::new();
    let mut skipped = Vec::new();
    for entry in walker {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(DiscoveryError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                skipped.push(SkippedFile {
                    absolute_path: root.to_path_buf(),
                    reason: DiscoveryError::Walk(error).to_string(),
                });
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let absolute_path = entry.path().to_path_buf();
        let relative_path = match strip_project_prefix(root, &absolute_path) {
            Ok(relative_path) => relative_path,
            Err(skipped_file) => {
                skipped.push(skipped_file);
                continue;
            }
        };
        if is_excluded(relative_path) {
            continue;
        }
        let kind = match sniff_kind(&absolute_path) {
            Ok(kind) => kind,
            Err(error) => {
                skipped.push(SkippedFile {
                    reason: error.to_string(),
                    absolute_path,
                });
                continue;
            }
        };
        let Some(relative_path) = relative_path.to_str() else {
            skipped.push(SkippedFile {
                reason: DiscoveryError::NonUtf8Path(absolute_path.clone()).to_string(),
                absolute_path,
            });
            continue;
        };
        discovered.push(DiscoveredFile {
            relative_path: relative_path.replace('\\', "/"),
            absolute_path,
            kind,
        });
    }
    discovered.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    skipped.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
    Ok((discovered, skipped))
}

/// The metadata of `absolute_path` when it is a regular file, `None` when it
/// is missing or anything else. `symlink_metadata`, not `metadata`: discovery
/// never follows symlinks, so an incremental re-index must not either, or it
/// would index a target — possibly outside the project — a full publish skips.
pub(crate) fn regular_file_metadata(
    absolute_path: &Path,
) -> Result<Option<std::fs::Metadata>, super::hash::HashError> {
    match std::fs::symlink_metadata(absolute_path) {
        Ok(metadata) => Ok(metadata.is_file().then_some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(super::hash::HashError::Read {
            path: absolute_path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
