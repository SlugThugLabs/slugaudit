//! Matcher assembly for [`super::IgnoreRules::build`]: grouping the project's
//! ignore files by directory and compiling one `Gitignore` matcher per
//! directory per precedence class. Kept separate from the rules type and its
//! query path so each file states one concern; the precedence and
//! descend-and-prune semantics live in the parent module's docs.

use crate::util::redacted_path;
use crate::watch::errors::ignore_error_kind;
use ignore::Error;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The ignore-file class a build failure came from. Static, so a diagnostic
/// identifies the source without logging any user path.
#[derive(Clone, Copy)]
enum Source {
    ProjectGitignore,
    ProjectIgnore,
    GlobalGitignore,
    GitExclude,
}

impl Source {
    const fn as_str(self) -> &'static str {
        match self {
            Source::ProjectGitignore => "project_gitignore",
            Source::ProjectIgnore => "project_ignore",
            Source::GlobalGitignore => "global_gitignore",
            Source::GitExclude => "git_exclude",
        }
    }
}

/// Logs an `ignore::Error` without its `Display` form.
///
/// `Display` for `WithPath`/`WithLineNumber`/`Glob` embeds the walker's
/// absolute path and the offending ignore-file line, and `Loop` embeds both
/// paths (ignore 0.4.33, `lib.rs:338-343`), so formatting the error would put
/// an absolute project or home path and user ignore-file content into the
/// server log. The record carries static `source`/`error_kind` fields and a
/// non-reversible `path_id` (plus `ancestor_id` for a loop) for correlation.
///
/// `Error::Partial` joins its members with newlines, so each member is logged
/// as its own one-line record instead of one multi-line field.
fn log_build_failure(source: Source, context: Option<&Path>, error: &Error) {
    match error {
        Error::Partial(errors) => {
            for error in errors {
                log_build_failure(source, context, error);
            }
        }
        Error::WithLineNumber { err, .. } | Error::WithDepth { err, .. } => {
            log_build_failure(source, context, err);
        }
        Error::WithPath { path, err } => {
            log_one_failure(source, Some(path), ignore_error_kind(err))
        }
        Error::Loop { ancestor, child } => tracing::warn!(
            operation = "ignore_rules_build",
            source = source.as_str(),
            error_kind = "loop",
            path_id = ?Some(redacted_path(child)),
            ancestor_id = %redacted_path(ancestor),
            "failed to build ignore rules; the affected rules are not applied",
        ),
        other => log_one_failure(source, context, ignore_error_kind(other)),
    }
}

fn log_one_failure(source: Source, path: Option<&Path>, error_kind: &'static str) {
    let path_id = path.map(redacted_path);
    tracing::warn!(
        operation = "ignore_rules_build",
        source = source.as_str(),
        error_kind = error_kind,
        path_id = ?path_id,
        "failed to build ignore rules; the affected rules are not applied",
    );
}

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
    // Matcher diagnostics carry a static source and error category plus a
    // redacted path id: the offending ignore file is correlatable, and neither
    // the operator's home directory nor the project path enters a server log
    // (the blanket rule in `util::redacted_path` docs).
    for (dir, (gitignores, ignores)) in group_files_by_dir(ignore_files) {
        if let Some(matcher) = compile_ignore_matcher(&dir, &ignores) {
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
        log_build_failure(Source::GlobalGitignore, None, &error);
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
                path_id = %redacted_path(file),
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
fn compile_ignore_matcher(dir: &Path, files: &[PathBuf]) -> Option<Gitignore> {
    if files.is_empty() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    add_files(&mut builder, files, Source::ProjectIgnore);
    builder
        .build()
        .map_err(|err| log_build_failure(Source::ProjectIgnore, Some(dir), &err))
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
                    log_build_failure(Source::GitExclude, Some(&exclude), &err);
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
    add_files(&mut builder, files, Source::ProjectGitignore);
    builder
        .build()
        .map_err(|err| log_build_failure(Source::ProjectGitignore, Some(dir), &err))
        .ok()
}

/// Adds each file to the builder, warning on per-file failures (a malformed
/// ignore file degrades to being skipped; the walker would refuse it too).
fn add_files(builder: &mut GitignoreBuilder, files: &[PathBuf], source: Source) {
    for file in files {
        if let Some(err) = builder.add(file) {
            log_build_failure(source, Some(file), &err);
        }
    }
}
