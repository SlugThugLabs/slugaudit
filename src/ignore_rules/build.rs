//! Matcher assembly for [`super::IgnoreRules::build`]: grouping the project's
//! ignore files by directory and compiling one `Gitignore` matcher per
//! directory per precedence class. Kept separate from the rules type and its
//! query path so each file states one concern; the precedence and
//! descend-and-prune semantics live in the parent module's docs.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Compiles every per-directory matcher plus the global gitignore.
/// Returns the two precedence classes (`.ignore` first, `.gitignore`
/// second) and the global matcher; root entries are seeded even when
/// empty, so the root is always matched against.
pub(super) fn assemble_matchers(
    root: &Path,
    ignore_files: &[PathBuf],
) -> (
    HashMap<PathBuf, Gitignore>,
    HashMap<PathBuf, Gitignore>,
    Gitignore,
) {
    let mut ignore_by_dir: HashMap<PathBuf, Gitignore> = HashMap::new();
    let mut gitignore_by_dir: HashMap<PathBuf, Gitignore> = HashMap::new();
    // Matcher diagnostics log the project-relative path: the offending
    // ignore file is fully identified, and the operator's home directory
    // never enters a server log (the blanket rule in `util::redacted_path`
    // docs; relative form keeps the line diagnosable, which a hash would not).
    for (dir, (gitignores, ignores)) in group_files_by_dir(ignore_files) {
        if let Some(matcher) = compile_ignore_matcher(root, &dir, &ignores) {
            ignore_by_dir.insert(dir.clone(), matcher);
        }
        if let Some(matcher) = compile_gitignore_matcher(root, &dir, &gitignores) {
            gitignore_by_dir.insert(dir, matcher);
        }
    }
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
    (ignore_by_dir, gitignore_by_dir, global)
}

/// Group by parent directory and by file type so each directory gets one
/// `.ignore` matcher and one `.gitignore` matcher — the two precedence
/// classes the walker keeps apart.
pub(super) fn group_files_by_dir(
    ignore_files: &[PathBuf],
) -> HashMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)> {
    let mut files_by_dir: HashMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)> = HashMap::new();
    for file in ignore_files {
        let Some(parent) = file.parent() else {
            tracing::warn!(
                path = %file.display(),
                "ignore file has no parent directory; its rules are not applied",
            );
            continue;
        };
        let (gitignores, ignores) = files_by_dir.entry(parent.to_path_buf()).or_default();
        if file.file_name().and_then(|n| n.to_str()) == Some(".ignore") {
            ignores.push(file.clone());
        } else {
            gitignores.push(file.clone());
        }
    }
    files_by_dir
}

/// Compiles the `.ignore` matcher for one directory; `None` (with the failure
/// warned) when there are no files or the matcher will not compile — the
/// directory then falls back to its ancestors, matching the walker.
fn compile_ignore_matcher(root: &Path, dir: &Path, files: &[PathBuf]) -> Option<Gitignore> {
    if files.is_empty() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    add_files(root, &mut builder, files, ".ignore");
    builder
        .build()
        .map_err(|err| {
            tracing::warn!(
                dir = %rel(root, dir),
                error = %err,
                "failed to compile .ignore matcher",
            );
        })
        .ok()
}

/// Compiles the `.gitignore` matcher for one directory. The root's matcher
/// additionally carries `.git/info/exclude`, added before the root
/// `.gitignore` files so it stays lowest-precedence within the class, per
/// git semantics.
fn compile_gitignore_matcher(root: &Path, dir: &Path, files: &[PathBuf]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(dir);
    if dir == root {
        let exclude = root.join(".git/info/exclude");
        match exclude.try_exists() {
            Ok(true) => {
                if let Some(err) = builder.add(&exclude) {
                    tracing::warn!(
                        path = ".git/info/exclude",
                        error = %err,
                        "failed to add .git/info/exclude to matcher",
                    );
                }
            }
            Ok(false) => {}
            Err(err) => tracing::warn!(
                path = ".git/info/exclude",
                error = %err,
                "could not stat .git/info/exclude; its rules are not applied",
            ),
        }
    }
    add_files(root, &mut builder, files, ".gitignore");
    builder
        .build()
        .map_err(|err| {
            tracing::warn!(
                dir = %rel(root, dir),
                error = %err,
                "failed to compile .gitignore matcher",
            );
        })
        .ok()
}

/// Adds each file to the builder, warning on per-file failures (a malformed
/// ignore file degrades to being skipped; the walker would refuse it too).
fn add_files(root: &Path, builder: &mut GitignoreBuilder, files: &[PathBuf], class: &str) {
    for file in files {
        if let Some(err) = builder.add(file) {
            tracing::warn!(
                path = %rel(root, file),
                error = %err,
                "failed to add {class} file to matcher",
            );
        }
    }
}

/// Project-relative display form for a path under `root` (see the
/// redaction note in [`assemble_matchers`]).
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
