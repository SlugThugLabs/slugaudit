// slugaudit-line-exception: approved-by=slugthug; reason=Recombined filesystem discovery walk and entry collection into cohesive module.
use crate::ignore_rules::{indexable_walker, is_excluded_path};
use crate::model::ResourceLimits;
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
    /// Text file whose bytes are read, hashed, and parsed.
    Indexed,
    /// Binary file: hashed for identity, but its content is not retained.
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One file yielded by the discovery walk: relative and absolute paths, kind,
/// and the size/mtime signals the sample stage re-checks.
pub struct DiscoveredFile {
    /// Project-relative path with `/` separators.
    pub relative_path: String,
    /// Absolute path used to read the file.
    pub absolute_path: PathBuf,
    /// Binary/text classification from the walk's sniff.
    pub kind: FileKind,
}

/// A file the walk found but could not include, with why. Discovery
/// continues past these rather than failing the entire project — one
/// unreadable or non-UTF8-named file must not make every file in the
/// project unqueryable. Never produced for a walk-level error: a directory
/// (or the root) that could not be listed fails discovery instead, because
/// its absence is indistinguishable from deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    /// Absolute path of the skipped file.
    pub absolute_path: PathBuf,
    /// Why the entry was skipped, as carried into the publish report and log.
    pub reason: String,
}

#[derive(Debug, Error)]
/// Walk-level failures. Unrepresentable or unreadable *files* become
/// reported `skipped` entries; a walk error or an ignore file the walker
/// could not apply aborts discovery with the reason, because those leave
/// the walked set incomplete in a way no skip entry can represent.
pub enum DiscoveryError {
    /// The walker could not list a directory (or the project root).
    #[error("failed to walk project files: {0}")]
    Walk(#[from] ignore::Error),
    /// The walker could not apply an ignore file: its rules were dropped,
    /// so the walked set may include paths the project meant to exclude.
    /// Fails closed rather than index a set the ignore rules refute.
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
    /// A discovered file could not be read while sniffing it.
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
    /// Relative paths are stored and queried as UTF-8 text. A non-UTF-8
    /// path is rejected rather than lossily rewritten, which would make
    /// the stored path unusable for round-tripping back to disk.
    #[error("discovered path {0} is not valid UTF-8")]
    NonUtf8Path(PathBuf),
    /// The walk exceeded its wall-clock deadline.
    #[error("discovery exceeded its wall-clock time budget after {elapsed_ms} ms")]
    TimeBudgetExceeded {
        /// Elapsed wall-clock milliseconds when the deadline tripped.
        elapsed_ms: u128,
    },
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
    is_excluded_path(&relative.to_string_lossy())
}

pub(super) fn strip_project_prefix<'a>(
    root: &Path,
    absolute_path: &'a Path,
) -> Result<&'a Path, SkippedFile> {
    absolute_path.strip_prefix(root).map_err(|_| SkippedFile {
        reason: DiscoveryError::NotRelative(absolute_path.to_path_buf()).to_string(),
        absolute_path: absolute_path.to_path_buf(),
    })
}

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
/// The walk runs under the default sync wall-clock budget
/// ([`ResourceLimits::default`]); callers that hold the effective limits use
/// [`discover_with_deadline`].
///
/// # Errors
///
/// Returns an error if the walk cannot list a directory (or the root), or
/// cannot apply an ignore file. A partial walked set is not a file set — a
/// publish would read the missing paths as deleted — so discovery fails
/// closed instead.
pub fn discover(root: &Path) -> Result<(Vec<DiscoveredFile>, Vec<SkippedFile>), DiscoveryError> {
    discover_with_deadline(
        root,
        &Deadline::after(ResourceLimits::default().max_sync_wall_clock),
    )
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
        collect_walk_entry(root, entry, &mut discovered, &mut skipped)?;
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
