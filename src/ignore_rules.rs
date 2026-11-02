//! Shared exclusions for SlugAudit's index.
//!
//! One source of truth for "what must not be indexed": the hardcoded
//! exclusions (the tool's own data directory, VCS internals, scratch
//! files) and the walker configuration that applies the project's ignore
//! files (`.gitignore`, `.ignore`, `.git/info/exclude`, and the global
//! gitignore). Discovery and every sync consult this module.

use std::path::Path;

/// SlugAudit's own data directory inside a project root (the project
/// database lives here). Never indexed.
pub const ACTIVATION_RELATIVE_DIR: &str = ".planning/slugaudit";
const EXCLUDED_COMPONENT: &str = ".git";

/// Directory-level hard exclusions: VCS internals and SlugAudit's own
/// data directory. Scratch-file rules apply only to files, matching
/// discovery.
pub(crate) fn is_excluded_dir(relative: &str) -> bool {
    is_activation_dir_or_descendant(relative)
        || relative
            .split(['/', '\\'])
            .any(|component| component == EXCLUDED_COMPONENT)
}

/// True for exactly the runtime data directory and its descendants. Matches
/// whole path components, so a sibling that merely shares the textual prefix
/// (`.planning/slugaudit-notes/`, `.planning/slugauditx`) stays indexed.
/// Accepts both `/` and `\` separators, like the `.git` check.
fn is_activation_dir_or_descendant(relative: &str) -> bool {
    let mut components = relative.split(['/', '\\']);
    ACTIVATION_RELATIVE_DIR
        .split('/')
        .all(|expected| components.next() == Some(expected))
}

/// Scratch-file suffixes, compared byte for byte: `NOTES.TMP` is not scratch.
const SCRATCH_SUFFIXES: [&str; 4] = [".tmp", ".bak", ".swp", "~"];

/// True if the file name (last component) is scratch/editor junk that is
/// almost never legitimate project source. Mirrors `sync::discovery`'s
/// heuristic exactly.
fn is_scratch_file_name(relative: &Path) -> bool {
    let Some(file_name) = relative.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    file_name.ends_with(".claude_output.txt")
        || file_name.starts_with("scratch.")
        || SCRATCH_SUFFIXES
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
}

/// The hardcoded exclusion set, shared by discovery and every sync pass.
#[must_use]
pub fn is_excluded_path(relative: &str) -> bool {
    is_excluded_dir(relative) || is_scratch_file_name(Path::new(relative))
}

/// The `ignore::Error` variant as a static, path-free category.
///
/// `Display` for `WithPath` is `{path}: {err}` and `Loop` embeds both paths
/// (ignore 0.4.33, `lib.rs:338-340`), so a walker error is never formatted
/// into a log line.
pub(crate) fn ignore_error_kind(error: &ignore::Error) -> &'static str {
    match error {
        ignore::Error::WithLineNumber { err, .. }
        | ignore::Error::WithDepth { err, .. }
        | ignore::Error::WithPath { err, .. } => ignore_error_kind(err),
        ignore::Error::Partial(_) => "partial",
        ignore::Error::Loop { .. } => "loop",
        ignore::Error::Io(_) => "io",
        ignore::Error::Glob { .. } => "glob",
        ignore::Error::UnrecognizedFileType(_) => "unrecognized_file_type",
        ignore::Error::InvalidDefinition => "invalid_definition",
    }
}

/// The path an `ignore::Error` carries, if any, for correlation in a log
/// record.
pub(crate) fn ignore_error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            ignore_error_path(err)
        }
        _ => None,
    }
}

/// A walker with exactly the configuration `sync::discovery` uses:
/// standard ignore filters on, hidden files included, symlinks not
/// followed. Discovery builds its walker from here so the indexable set
/// is defined in one place.
pub(crate) fn indexable_walker(root: &Path) -> ignore::Walk {
    indexable_walker_with_max_depth(root, None)
}

/// [`indexable_walker`], optionally bounded at `max_depth` (the root is
/// depth 0). Discovery uses the unbounded form; a caller may bound depth so
/// a pathologically deep tree cannot dictate how long the walk runs. The
/// ignore filters are identical either way — only the descent limit
/// differs.
pub(crate) fn indexable_walker_with_max_depth(
    root: &Path,
    max_depth: Option<usize>,
) -> ignore::Walk {
    let mut builder = ignore::WalkBuilder::new(root);
    let filter_root = root.to_path_buf();
    builder
        .standard_filters(true)
        // `standard_filters` enables `parents`, which makes the walker apply
        // `.gitignore`/`.ignore` files above the project root. Project-scoped
        // rules are the shared contract.
        .parents(false)
        .follow_links(false)
        // Hidden files stay indexable (`.github/`, `.env.example`); VCS
        // internals and SlugAudit's own data directory are pruned below
        // instead of being walked and filtered per file.
        .hidden(false)
        .require_git(false)
        // Prune excluded directories at descent time. Without this the walk
        // still pays for `.git` and `.planning/slugaudit`, and a large
        // object store or database journal slows every sync.
        .filter_entry(move |entry| {
            let relative = match entry.path().strip_prefix(&filter_root) {
                Ok(relative) => relative,
                Err(error) => {
                    // Continuing is correct: an entry outside the root is kept
                    // (not pruned) so discovery sees it, and discovery records
                    // the same `strip_prefix` failure in its `skipped` list.
                    tracing::warn!(
                        path = %crate::util::redacted_path(entry.path()),
                        error = %error,
                        "walk entry is outside the project root; not pruning it",
                    );
                    return true;
                }
            };
            !is_excluded_dir(&relative.to_string_lossy())
        });
    if let Some(max_depth) = max_depth {
        builder.max_depth(Some(max_depth));
    }
    builder.build()
}

#[cfg(test)]
#[path = "ignore_rules_tests.rs"]
mod tests;
