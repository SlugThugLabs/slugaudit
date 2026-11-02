//! Shared ignore rules for SlugAudit's index.
//!
//! One source of truth for "what must not be indexed": the hardcoded
//! exclusions (the tool's own data directory, VCS internals, scratch
//! files) and the project's ignore files (`.gitignore`, `.ignore`,
//! `.git/info/exclude`, and the global gitignore). Discovery and every
//! sync consult this module, so the walker and the matcher can never
//! disagree about the same tree.
//!
//! The matcher mirrors the `ignore` crate's walker semantics. A single
//! `GitignoreBuilder` cannot hold nested `.gitignore` files: patterns are
//! stored without their base directory and would leak outside it, so each
//! directory that has an ignore file gets its own matcher, keyed by
//! absolute path, and matching walks top-down to reproduce the walker's
//! descend-and-prune behavior. Precedence is type-based first, exactly as
//! the crate documents: any `.ignore` file overrides all `.gitignore`
//! files regardless of directory depth, so `.ignore` matchers are checked
//! before `.gitignore` matchers; within each class, deeper directories win.

use ignore::Match;
use ignore::gitignore::Gitignore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
            let relative = entry
                .path()
                .strip_prefix(&filter_root)
                .unwrap_or_else(|_| entry.path());
            !is_excluded_dir(&relative.to_string_lossy())
        });
    if let Some(max_depth) = max_depth {
        builder.max_depth(Some(max_depth));
    }
    builder.build()
}

/// Ignore rules for one project root: the hardcoded exclusions plus the
/// project's ignore files, scoped per directory.
#[derive(Debug, Clone)]
pub struct IgnoreRules {
    root: PathBuf,
    /// `.ignore` matchers keyed by absolute directory path — the highest
    /// precedence class: any `.ignore` overrides all `.gitignore` files.
    ignore_by_dir: HashMap<PathBuf, Gitignore>,
    /// `.gitignore` matchers keyed by absolute directory path; the root
    /// entry also carries `.git/info/exclude` (lowest within its class).
    gitignore_by_dir: HashMap<PathBuf, Gitignore>,
    /// The global gitignore (lowest precedence of all).
    global: Gitignore,
}

mod build;

impl IgnoreRules {
    /// Builds rules for `root` from `.gitignore`/`.ignore` files under it
    /// (typically collected from the same walk that enumerated the
    /// indexable directories). Each file is scoped to its own directory;
    /// within a directory, `.ignore` overrides `.gitignore` because the
    /// crate resolves the last matching pattern and the files are added
    /// `.gitignore`-first.
    ///
    /// Matcher assembly lives in `build`; this is the thin constructor.
    #[must_use]
    pub fn build(root: &Path, ignore_files: &[PathBuf]) -> IgnoreRules {
        let (ignore_by_dir, gitignore_by_dir, global) =
            build::assemble_matchers(root, ignore_files);
        IgnoreRules {
            root: root.to_path_buf(),
            ignore_by_dir,
            gitignore_by_dir,
            global,
        }
    }

    /// True if `relative` (a project-relative, `/`-separated path) must
    /// not be indexed: a hardcoded exclusion or a matching ignore rule.
    #[must_use]
    pub fn should_ignore(&self, relative: &str) -> bool {
        if is_excluded_path(relative) {
            return true;
        }
        self.is_ignored_by_rules(relative)
    }

    /// Matches `relative` against the ignore-file rules only, walking
    /// top-down to mirror the walker's descend-and-prune: an ignored
    /// directory rules out everything below it (git cannot re-include a
    /// file under an excluded parent), and a whitelisted directory
    /// re-includes its subtree.
    fn is_ignored_by_rules(&self, relative: &str) -> bool {
        let mut current = self.root.clone();
        let mut components = relative
            .split(['/', '\\'])
            .filter(|c| !c.is_empty())
            .peekable();
        while let Some(component) = components.next() {
            current.push(component);
            let is_last = components.peek().is_none();
            match self.matched_at(&current, /* is_dir */ !is_last) {
                Match::Ignore(()) => return true,
                Match::Whitelist(()) | Match::None => {}
            }
        }
        false
    }

    /// Checks `path` against the applicable matchers and returns the first
    /// decisive verdict. Precedence is type-based first, mirroring the
    /// walker: every `.ignore` matcher (deepest directory first) is checked
    /// before any `.gitignore` matcher, then the global gitignore last.
    fn matched_at(&self, path: &Path, is_dir: bool) -> Match<()> {
        if let Some(verdict) = Self::chain_match(&self.ignore_by_dir, path, is_dir, &self.root) {
            return verdict;
        }
        if let Some(verdict) = Self::chain_match(&self.gitignore_by_dir, path, is_dir, &self.root) {
            return verdict;
        }
        match self.global.matched(path, is_dir) {
            Match::Ignore(_) => Match::Ignore(()),
            Match::Whitelist(_) => Match::Whitelist(()),
            Match::None => Match::None,
        }
    }

    /// Checks one precedence class of per-directory matchers, walking from
    /// the path's own directory up to the root (deeper directories win)
    /// and returning the first decisive verdict.
    fn chain_match(
        matchers: &HashMap<PathBuf, Gitignore>,
        path: &Path,
        is_dir: bool,
        root: &Path,
    ) -> Option<Match<()>> {
        let mut dir = path.parent().unwrap_or(root).to_path_buf();
        loop {
            if let Some(matcher) = matchers.get(&dir) {
                match matcher.matched(path, is_dir) {
                    Match::Ignore(_) => return Some(Match::Ignore(())),
                    Match::Whitelist(_) => return Some(Match::Whitelist(())),
                    Match::None => {}
                }
            }
            if dir == root {
                return None;
            }
            if !dir.pop() {
                return None;
            }
        }
    }
}

#[cfg(test)]
#[path = "ignore_rules_tests.rs"]
mod tests;
