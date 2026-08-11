use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use thiserror::Error;

const ACTIVATION_RELATIVE_DIR: &str = ".planning/slugaudit";
const EXCLUDED_COMPONENT: &str = ".git";
/// Bytes sampled from the start of a file to decide binary vs. text, same
/// heuristic class as git/ripgrep: a NUL byte in the sample means binary.
const BINARY_SNIFF_BYTES: usize = 8_000;

/// Scratch, temp, and editor-swap file suffixes/prefixes that are almost
/// never legitimate project source. Their parse failures would otherwise
/// pollute the project's evidence with noise from AI-generated session
/// output and editor backups — things that exist transiently on disk but
/// aren't source the project intends to maintain. Checked against the
/// file name (last component) only, so a real file named `scratch.py` in
/// a subdirectory is still excluded (vanishingly rare), while anyone who
/// genuinely needs one can override via `.gitignore`.
fn is_scratch_file(relative: &Path) -> bool {
    let Some(file_name) = relative.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    file_name.ends_with(".claude_output.txt")
        || file_name.starts_with("scratch.")
        || file_name.ends_with(".tmp")
        || file_name.ends_with(".bak")
        || file_name.ends_with(".swp")
        || file_name.ends_with('~')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Indexed,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

fn is_excluded(relative: &Path) -> bool {
    if relative.starts_with(ACTIVATION_RELATIVE_DIR) {
        return true;
    }
    relative
        .components()
        .any(|component| component.as_os_str() == EXCLUDED_COMPONENT)
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
    let mut buffer = vec![0_u8; BINARY_SNIFF_BYTES];
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
    // `standard_filters` sets several flags at once, including `hidden`; it
    // must run before the explicit `hidden(false)` override or it silently
    // wins and dotfiles/dotdirs (e.g. `.github/`) vanish from the walk.
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .follow_links(false)
        .hidden(false)
        .build();

    let mut discovered = Vec::new();
    let mut skipped = Vec::new();
    for entry in walker {
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
        let relative_path = match absolute_path.strip_prefix(root) {
            Ok(relative_path) => relative_path,
            Err(_) => {
                skipped.push(SkippedFile {
                    reason: DiscoveryError::NotRelative(absolute_path.clone()).to_string(),
                    absolute_path,
                });
                continue;
            }
        };
        if is_excluded(relative_path) {
            continue;
        }
        if is_scratch_file(relative_path) {
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

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
