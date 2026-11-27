// slugaudit-line-exception: approved-by=split-judge; reason=filesystem discovery walk, UTF-8/raw-byte path classification, and single-open Pass 1 walk+hash entry point
//! Filesystem discovery walk, UTF-8 path classification, and single-pass
//! discovery + hashing entry points.

use crate::ignore_rules::{indexable_walker, is_excluded_path};
#[cfg(test)]
use crate::model::ResourceLimits;
use crate::util::Deadline;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Bytes sampled from the start of a file to decide binary vs. text, same
/// heuristic class as git/ripgrep: a NUL byte in the sample means binary.
pub(super) const BINARY_SNIFF_BYTES: usize = 8_000;

/// A regular file yielded by the directory walk prior to opening or reading
/// its bytes, so Pass 1 can sniff and BLAKE3-hash in one parallel file open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WalkedFile {
    /// Project-relative path with `/` separators.
    pub(super) relative_path: String,
    /// Absolute path used to read the file.
    pub(super) absolute_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// What Pass 1 concluded about a file before sampling: indexable text or
/// binary (content not retained).
pub enum FileKind {
    /// Text file whose bytes are read, hashed, and parsed.
    Indexed,
    /// Binary file: hashed for identity, but its content is not retained.
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One file yielded by discovery and Pass 1 sniffing: relative and absolute
/// paths plus binary/text classification.
pub struct DiscoveredFile {
    /// Project-relative path with `/` separators.
    pub relative_path: String,
    /// Absolute path used to read the file.
    pub absolute_path: PathBuf,
    /// Binary/text classification from Pass 1's sniff.
    pub kind: FileKind,
}

/// A file discovery or sampling found on disk but could not index, with why.
///
/// One unreadable, oversized, or non-UTF8-named file is recorded here rather
/// than failing the entire project. Walk-level failures (an unlistable root or
/// subdirectory, or an ignore file that could not be parsed) return
/// [`DiscoveryError`] instead so a partial walk never looks like deletions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    /// Absolute path of the skipped file.
    pub absolute_path: PathBuf,
    /// Why the entry was skipped, as carried into the publish report and log.
    pub reason: String,
    /// Stable `files.skip_reason` code for a representable skip
    /// (`unreadable`, `oversize`, `not_utf8`).
    pub code: &'static str,
}

impl SkippedFile {
    pub(super) fn sort_by_path(skips: &mut [Self]) {
        skips.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
    }
}

#[derive(Debug, Error)]
/// Walk-level failures and typed per-file skip reasons.
pub enum DiscoveryError {
    /// The walker could not list a directory (or the project root).
    #[error("failed to walk project files: {0}")]
    Walk(#[from] ignore::Error),
    /// The walker could not apply an ignore file: its rules were dropped,
    /// so the walked set may include paths the project meant to exclude.
    #[error(
        "failed to apply the project's ignore rules ({kind}); refusing to index a file set \
         those rules may exclude"
    )]
    IgnoreRules {
        /// Static `ignore::Error` category, never its path-bearing Display.
        kind: &'static str,
        /// The ignore file the walker could not apply, when it named one.
        path: Option<PathBuf>,
    },
    /// A discovered file could not be read during Pass 1 sniffing/hashing.
    #[error("failed to read {path}: {source}")]
    Read {
        /// Absolute path of the unreadable file.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The walker yielded a path outside the project root.
    #[error("discovered path {0} is not relative to the project root")]
    NotRelative(PathBuf),
    /// Relative paths are stored and queried as UTF-8 text; a non-UTF-8 path
    /// is skipped rather than lossily rewritten.
    #[error("discovered path {0} is not valid UTF-8")]
    NonUtf8Path(PathBuf),
    /// A file the walk listed was deleted before it was read.
    #[error("{relative_path} was deleted during the walk")]
    Vanished {
        /// Project-relative path of the file, as the walk listed it.
        relative_path: String,
    },
    /// The walk exceeded its wall-clock deadline.
    #[error("discovery exceeded its wall-clock time budget after {elapsed_ms} ms")]
    TimeBudgetExceeded {
        /// Elapsed wall-clock milliseconds when the deadline tripped.
        elapsed_ms: u128,
    },
}

impl DiscoveryError {
    /// Elapsed budget when this error is a wall-clock budget exhaustion.
    #[must_use]
    pub(crate) fn budget_elapsed_ms(&self) -> Option<u128> {
        match self {
            DiscoveryError::TimeBudgetExceeded { elapsed_ms } => Some(*elapsed_ms),
            _ => None,
        }
    }
}

/// Whether a read of a walked file failed because the file vanished (`NotFound`).
#[must_use]
pub(super) fn is_vanished(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

fn normalize_slashes(relative: &str) -> String {
    relative.replace('\\', "/")
}

fn non_empty_relative<'a>(root: &Path, absolute_path: &'a Path) -> Option<&'a Path> {
    let Ok(relative) = absolute_path.strip_prefix(root) else {
        return None;
    };
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative)
}

pub(super) fn strip_project_prefix<'a>(
    root: &Path,
    absolute_path: &'a Path,
) -> Result<&'a Path, SkippedFile> {
    absolute_path.strip_prefix(root).map_err(|_| SkippedFile {
        reason: DiscoveryError::NotRelative(absolute_path.to_path_buf()).to_string(),
        absolute_path: absolute_path.to_path_buf(),
        code: super::revision::SKIP_REASON_UNREADABLE,
    })
}

/// Validates one walker entry and appends regular indexable files to `entries`
/// or non-UTF-8 / out-of-root paths to `skipped`.
pub(super) fn collect_walked_entry(
    root: &Path,
    entry: Result<ignore::DirEntry, ignore::Error>,
    entries: &mut Vec<WalkedFile>,
    skipped: &mut Vec<SkippedFile>,
) -> Result<(), DiscoveryError> {
    let entry = entry.map_err(DiscoveryError::Walk)?;
    if let Some(error) = entry.error() {
        return Err(DiscoveryError::IgnoreRules {
            kind: crate::ignore_rules::ignore_error_kind(error),
            path: crate::ignore_rules::ignore_error_path(error).map(Path::to_path_buf),
        });
    }
    if !entry.file_type().is_some_and(|kind| kind.is_file()) {
        return Ok(());
    }
    let absolute_path = entry.path().to_path_buf();
    let relative_path = match strip_project_prefix(root, &absolute_path) {
        Ok(relative_path) => relative_path,
        Err(skipped_file) => {
            skipped.push(skipped_file);
            return Ok(());
        }
    };
    if is_excluded_path(&relative_path.to_string_lossy()) {
        return Ok(());
    }
    match relative_path.to_str() {
        Some(relative_path) => entries.push(WalkedFile {
            relative_path: normalize_slashes(relative_path),
            absolute_path,
        }),
        None => skipped.push(SkippedFile {
            reason: DiscoveryError::NonUtf8Path(absolute_path.clone()).to_string(),
            absolute_path,
            code: super::revision::SKIP_REASON_UNREADABLE,
        }),
    }
    Ok(())
}

/// Project-relative UTF-8 path of a skipped file, or `None` when the path is
/// non-UTF-8, outside `root`, or equal to `root` itself.
#[must_use]
pub(super) fn representable_relative_path(root: &Path, absolute_path: &Path) -> Option<String> {
    non_empty_relative(root, absolute_path)?
        .to_str()
        .map(normalize_slashes)
}

/// Raw OS bytes of a discovered project-relative path that is not valid UTF-8.
#[must_use]
pub(super) fn unrepresentable_relative_bytes(root: &Path, absolute_path: &Path) -> Option<Vec<u8>> {
    let relative = non_empty_relative(root, absolute_path)?;
    if relative.to_str().is_some() {
        return None;
    }
    Some(relative.as_os_str().as_encoded_bytes().to_vec())
}

/// Unrepresentable non-UTF-8 paths among `skipped`, persisted in
/// `unrepresentable_paths`.
#[must_use]
pub(super) fn unrepresentable_paths(root: &Path, skipped: &[SkippedFile]) -> HashSet<Vec<u8>> {
    skipped
        .iter()
        .filter_map(|file| unrepresentable_relative_bytes(root, &file.absolute_path))
        .collect()
}

/// Whether `bytes` sniffs as binary (a NUL byte within the first
/// [`BINARY_SNIFF_BYTES`] bytes).
#[must_use]
pub(super) fn is_binary_bytes(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0)
}

/// Walks the directory tree applying ignore rules and UTF-8 path validation
/// without opening regular files, so Pass 1 (`sniff_and_hash_walked`) opens,
/// binary-sniffs, and BLAKE3-hashes each file once across worker threads.
pub(super) fn walk_with_deadline(
    root: &Path,
    deadline: &Deadline,
) -> Result<(Vec<WalkedFile>, Vec<SkippedFile>), DiscoveryError> {
    let walker = indexable_walker(root);
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for entry in walker {
        if let Some(elapsed) = deadline.exceeded() {
            return Err(DiscoveryError::TimeBudgetExceeded {
                elapsed_ms: elapsed.as_millis(),
            });
        }
        collect_walked_entry(root, entry, &mut entries, &mut skipped)?;
    }
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    SkippedFile::sort_by_path(&mut skipped);
    Ok((entries, skipped))
}

/// Runs the live Pass 1 discovery + single-open sniff/hash pipeline under an
/// explicit byte cap and wall-clock [`Deadline`].
#[cfg(test)]
pub(crate) fn discover_and_hash_with_deadline(
    root: &Path,
    max_file_bytes: u64,
    deadline: &Deadline,
) -> Result<super::hash_batch::SniffedWalk, super::publish::PublishError> {
    let (walked, mut walk_skips) = walk_with_deadline(root, deadline)?;
    let mut sniffed = super::hash_batch::sniff_and_hash_walked(&walked, max_file_bytes, deadline)?;
    walk_skips.extend(sniffed.skips);
    SkippedFile::sort_by_path(&mut walk_skips);
    sniffed.skips = walk_skips;
    Ok(sniffed)
}

/// Runs [`discover_and_hash_with_deadline`] under [`ResourceLimits::default`]
/// (test helper).
#[cfg(test)]
pub(crate) fn discover_and_hash(
    root: &Path,
) -> Result<(Vec<DiscoveredFile>, Vec<SkippedFile>), super::publish::PublishError> {
    let limits = ResourceLimits::default();
    let deadline = Deadline::after(limits.max_sync_wall_clock);
    let sniffed = discover_and_hash_with_deadline(root, limits.max_file_bytes, &deadline)?;
    Ok((sniffed.discovered, sniffed.skips))
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
