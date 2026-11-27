//! Project-root `.gitignore` helper for activation: appends `.planning/slugaudit/`
//! atomically unless already ignored, reporting [`GitignoreOutcome::NotWritten`]
//! instead of failing activation when `.gitignore` cannot be read or updated.

use super::activation::ProjectRoot;
use crate::ignore_rules::ACTIVATION_RELATIVE_DIR;
use crate::util::{
    CappedReadError, MAX_FOREIGN_TEXT_BYTES, read_file_capped_bytes, sync_directory,
    write_atomic_chunks_with,
};
use std::path::{Path, PathBuf};

pub(super) const GITIGNORE_FILE: &str = ".gitignore";

/// Outcome of [`ensure_gitignore`].
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreOutcome {
    /// The rule was appended (creating `.gitignore` when absent).
    Added(PathBuf),
    /// A rule ignoring `.planning/slugaudit` was already active; file untouched.
    AlreadyIgnored(PathBuf),
    /// `.gitignore` was left untouched; activation still succeeded.
    NotWritten {
        /// Path that was left untouched.
        path: PathBuf,
        /// Reason the rule was not written.
        reason: String,
    },
}

/// Appends `.planning/slugaudit/` to `<root>/.gitignore` unless already ignored.
#[must_use]
pub fn ensure_gitignore(root: &ProjectRoot) -> GitignoreOutcome {
    let path = root.as_path().join(GITIGNORE_FILE);
    let existing = match read_existing_gitignore(&path) {
        Ok(contents) => contents,
        Err(error) => {
            return GitignoreOutcome::NotWritten {
                path,
                reason: error.to_string(),
            };
        }
    };
    if ignores_activation_dir(&existing) {
        return GitignoreOutcome::AlreadyIgnored(path);
    }
    let newline = if existing.windows(2).any(|window| window == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let separator = if existing.is_empty() || existing.ends_with(b"\n") {
        ""
    } else {
        newline
    };
    let addition = format!("{separator}{ACTIVATION_RELATIVE_DIR}/{newline}");
    match write_gitignore_atomically(&path, &existing, addition.as_bytes(), sync_directory) {
        Ok(()) => GitignoreOutcome::Added(path),
        Err(error) => GitignoreOutcome::NotWritten {
            path,
            reason: error.to_string(),
        },
    }
}

fn read_existing_gitignore(path: &Path) -> Result<Vec<u8>, CappedReadError> {
    match read_file_capped_bytes(path, MAX_FOREIGN_TEXT_BYTES) {
        Err(CappedReadError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(Vec::new())
        }
        result => result,
    }
}

fn write_gitignore_atomically(
    path: &Path,
    existing: &[u8],
    addition: &[u8],
    sync_dir: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    write_atomic_chunks_with(
        path,
        &[existing, addition],
        std::fs::File::sync_all,
        sync_dir,
    )
}

fn ignores_activation_dir(contents: &[u8]) -> bool {
    let mut ignored = false;
    for line in String::from_utf8_lossy(contents).lines() {
        let rule = line.trim_end();
        if rule.starts_with('#') {
            continue;
        }
        let (negated, pattern) = match rule.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, rule),
        };
        let mut anchored = pattern;
        if let Some(rest) = pattern.strip_prefix('/') {
            anchored = rest;
        }
        if anchored.trim_end_matches('/') == ACTIVATION_RELATIVE_DIR {
            ignored = !negated;
        }
    }
    ignored
}

#[cfg(test)]
#[path = "gitignore_tests.rs"]
mod tests;
