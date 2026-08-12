//! Shared ignore rules for SlugAudit's index.
//!
//! One source of truth for "what must not be indexed": the hardcoded
//! exclusions (the tool's own data directory, VCS internals, scratch
//! files) and the project's ignore files (`.gitignore`, `.ignore`,
//! `.git/info/exclude`, and the global gitignore). Discovery (full
//! publish), the filesystem watcher (event filtering), and incremental
//! reconcile all consult this module so the full-publish file set and the
//! incremental file set can never drift apart — the watcher previously
//! indexed gitignored build artifacts (`target/`, ...) that a fresh
//! publish skipped.
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
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// SlugAudit's own data directory inside a project root (the project
/// database lives here). Never indexed, never watched.
pub const ACTIVATION_RELATIVE_DIR: &str = ".planning/slugaudit";
const EXCLUDED_COMPONENT: &str = ".git";

/// Directory-level hard exclusions: VCS internals and SlugAudit's own
/// data directory. Scratch-file rules apply only to files, matching
/// discovery. Crate-visible because the watch-scope walk filters
/// directories with it.
pub(crate) fn is_excluded_dir(relative: &str) -> bool {
    relative.starts_with(ACTIVATION_RELATIVE_DIR)
        || relative
            .split(['/', '\\'])
            .any(|component| component == EXCLUDED_COMPONENT)
}

/// True if the file name (last component) is scratch/editor junk that is
/// almost never legitimate project source. Mirrors `sync::discovery`'s
/// historical heuristic exactly.
fn is_scratch_file_name(relative: &Path) -> bool {
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

/// The hardcoded exclusion set, shared by discovery, the watcher, and
/// incremental reconcile.
pub fn is_excluded_path(relative: &str) -> bool {
    is_excluded_dir(relative) || is_scratch_file_name(Path::new(relative))
}

/// A walker with exactly the configuration `sync::discovery` uses:
/// standard ignore filters on, hidden files included, symlinks not
/// followed. Every consumer (discovery, watch-scope computation) builds
/// its walker from here so they agree on the indexable set by
/// construction.
pub(crate) fn indexable_walker(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .follow_links(false)
        .hidden(false)
        .build()
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

impl IgnoreRules {
    /// Builds rules for `root` from `.gitignore`/`.ignore` files under it
    /// (typically collected from the same walk that enumerated the
    /// indexable directories). Each file is scoped to its own directory;
    /// within a directory, `.ignore` overrides `.gitignore` because the
    /// crate resolves the last matching pattern and the files are added
    /// `.gitignore`-first.
    pub fn build(root: &Path, ignore_files: &[PathBuf]) -> IgnoreRules {
        // Group by parent directory and by file type so each directory gets
        // one `.ignore` matcher and one `.gitignore` matcher — the two
        // precedence classes the walker keeps apart.
        let mut files_by_dir: HashMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)> = HashMap::new();
        for file in ignore_files {
            if let Some(parent) = file.parent() {
                let (gitignores, ignores) = files_by_dir.entry(parent.to_path_buf()).or_default();
                let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == ".ignore" {
                    ignores.push(file.to_path_buf());
                } else {
                    gitignores.push(file.to_path_buf());
                }
            }
        }
        let mut ignore_by_dir: HashMap<PathBuf, Gitignore> = HashMap::new();
        let mut gitignore_by_dir: HashMap<PathBuf, Gitignore> = HashMap::new();
        for (dir, (gitignores, ignores)) in files_by_dir {
            if !ignores.is_empty() {
                let mut builder = GitignoreBuilder::new(&dir);
                for file in &ignores {
                    let _ = builder.add(file);
                }
                if let Ok(matcher) = builder.build() {
                    ignore_by_dir.insert(dir.clone(), matcher);
                }
            }
            // The root `.gitignore` matcher additionally carries
            // `.git/info/exclude`, lowest-precedence within its class per
            // git semantics.
            let mut builder = GitignoreBuilder::new(&dir);
            if dir == root {
                let exclude = root.join(".git/info/exclude");
                if exclude.is_file() {
                    let _ = builder.add(&exclude);
                }
            }
            for file in &gitignores {
                let _ = builder.add(file);
            }
            if let Ok(matcher) = builder.build() {
                gitignore_by_dir.insert(dir, matcher);
            }
        }
        // Ensure root entries exist even without ignore files, so the
        // root is always matched against (empty matchers match nothing).
        ignore_by_dir
            .entry(root.to_path_buf())
            .or_insert_with(Gitignore::empty);
        gitignore_by_dir
            .entry(root.to_path_buf())
            .or_insert_with(Gitignore::empty);

        let (global, error) = GitignoreBuilder::new(root).build_global();
        if let Some(error) = error {
            tracing::warn!(
                error = %error,
                "failed to load the global gitignore; continuing without it",
            );
        }

        IgnoreRules {
            root: root.to_path_buf(),
            ignore_by_dir,
            gitignore_by_dir,
            global,
        }
    }

    /// True if `relative` (a project-relative, `/`-separated path) must
    /// not be indexed: a hardcoded exclusion or a matching ignore rule.
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
                Match::Ignore(_) => return true,
                Match::Whitelist(_) | Match::None => {}
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
