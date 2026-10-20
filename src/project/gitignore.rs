//! The project-root `.gitignore` guard for activation: append the
//! `.planning/slugaudit/` rule unless it is already ignored, atomically and
//! without touching any other byte.
//!
//! Kept separate from the activation marker because the guard is non-fatal: a
//! failure here is reported as [`GitignoreOutcome::NotWritten`] and never
//! undoes an enable.

use super::root::ProjectRoot;
use crate::ignore_rules::ACTIVATION_RELATIVE_DIR;
use crate::util::{CappedReadError, MAX_FOREIGN_TEXT_BYTES, read_file_capped_bytes};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The ignore file at the project root. It belongs to the user: SlugAudit
/// only ever appends one rule to it and never rewrites or removes anything.
pub(super) const GITIGNORE_FILE: &str = ".gitignore";

/// What [`ensure_gitignore`] did, and to which file.
///
/// There is deliberately no error case here. The activation directory is
/// what makes a project enabled; a `.gitignore` that cannot be read or
/// written is *reported* to the caller, never fatal to the enable, because a
/// convenience guard must not undo the primary action.
///
/// The file is foreign, user-controlled text, so it is read under the same
/// 1 MiB cap as other foreign text files (`MAX_FOREIGN_TEXT_BYTES`). A
/// `.gitignore` over that cap is reported as [`Self::NotWritten`] rather than
/// read, truncated, or appended to; the project stays enabled either way.
///
/// `disable` deliberately does not remove the rule: a `.gitignore` rule can
/// be hiding paths SlugAudit knows nothing about, and un-hiding them is not
/// SlugAudit's call.
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreOutcome {
    /// The rule was appended; the file was created when it was absent.
    Added(PathBuf),
    /// A rule that already ignores the directory was present; file untouched.
    AlreadyIgnored(PathBuf),
    /// The file was left untouched, and the project is enabled regardless.
    NotWritten {
        /// The `.gitignore` path that was left untouched.
        path: PathBuf,
        /// Why the rule was not written, reported so the non-fatal failure is
        /// visible to the caller.
        reason: String,
    },
}

/// Appends the activation directory to the project-root `.gitignore` unless a
/// rule that already ignores it is present, so a customer's first
/// `git status` after `project_control {"action":"on"}` does not show the
/// index database and its sidecars as untracked files.
///
/// Every existing byte survives: the new contents are the old contents plus
/// the rule, a final line without a newline gets one first, and a file
/// written with CRLF keeps that convention instead of acquiring a mixed one.
/// The write is atomic — the complete contents go to a temp file in the same
/// directory (or beside a symlink's target, so a linked `.gitignore` stays a
/// link) which is then renamed over the path. A failed write therefore
/// leaves the file byte-identical, which is what
/// [`GitignoreOutcome::NotWritten`] promises; the trade-off is that a
/// concurrent writer's change made between the read and the rename is not
/// merged.
///
/// The file is read through `read_file_capped_bytes` with a 1 MiB cap
/// (`MAX_FOREIGN_TEXT_BYTES`), the same cap used for other foreign text
/// files. An over-cap file is reported as [`GitignoreOutcome::NotWritten`]
/// rather than read, and any other read failure is reported the same way:
/// this guard is non-fatal by design and never undoes the enable. A path
/// that is not a regular file (following a symlink to its target) is refused
/// before the read, because opening a FIFO would block forever, and a
/// dangling symlink is reported rather than replaced. Only a path with no
/// directory entry counts as missing; the atomic write creates it.
///
/// Failure is reported through [`GitignoreOutcome::NotWritten`], never
/// propagated — see [`GitignoreOutcome`].
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
    // Match the file's own line-ending convention rather than mixing one into
    // it, and give a newline-less final line its newline first.
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
    let entry = format!("{ACTIVATION_RELATIVE_DIR}/");
    let addition = format!("{separator}{entry}{newline}");
    match write_gitignore_atomically(&path, &existing, addition.as_bytes()) {
        Ok(()) => GitignoreOutcome::Added(path),
        Err(error) => GitignoreOutcome::NotWritten {
            path,
            reason: error.to_string(),
        },
    }
}

/// Reads the existing `.gitignore`, refusing anything that is not a regular
/// file before it can be opened.
///
/// `read_file_capped_bytes` bounds the read, not the open: `File::open` on a
/// FIFO blocks until a writer appears, so the path is classified first with
/// `symlink_metadata` and, for a link, its target's metadata. A symlink to a
/// regular file keeps working; a FIFO, directory, device, or a symlink to one
/// is refused, and a symlink whose target does not exist is reported rather
/// than replaced by a rename. A path with no directory entry is not a
/// failure: the atomic write creates it.
fn read_existing_gitignore(path: &Path) -> Result<Vec<u8>, CappedReadError> {
    let entry = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(CappedReadError::Io(error)),
    };
    let target = if entry.file_type().is_symlink() {
        match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CappedReadError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the .gitignore path is a symbolic link whose target does not exist",
                )));
            }
            Err(error) => return Err(CappedReadError::Io(error)),
        }
    } else {
        entry
    };
    if !target.is_file() {
        return Err(CappedReadError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the .gitignore path is not a regular file",
        )));
    }
    read_file_capped_bytes(path, MAX_FOREIGN_TEXT_BYTES)
}

/// Sequence making temp-file names unique within one process.
static GITIGNORE_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes the existing bytes plus `addition` to a temp file beside the real
/// `.gitignore` and renames it into place.
///
/// The destination is the symlink target when `.gitignore` is a symlink, so
/// the user's link survives; a missing file is created at the original path.
/// Nothing is renamed until the whole file has been written and synced, so a
/// failure before the rename — including a deferred write or commit error
/// surfaced by `sync_all` — leaves the original untouched. The temp file is
/// removed on failure; a cleanup failure is logged and never replaces the
/// write error that is already being reported.
fn write_gitignore_atomically(
    path: &Path,
    existing: &[u8],
    addition: &[u8],
) -> std::io::Result<()> {
    let destination = match std::fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let directory = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the .gitignore path has no parent directory",
        )
    })?;
    let sequence = GITIGNORE_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = directory.join(format!(
        ".gitignore.slugaudit.{}.{sequence}.tmp",
        std::process::id()
    ));

    // `create_new` fails rather than following a pre-existing path, so this
    // process can only ever clean up a temp file it created itself.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = file
        .write_all(existing)
        .and_then(|()| file.write_all(addition))
        .and_then(|()| match std::fs::metadata(&destination) {
            Ok(metadata) => file.set_permissions(metadata.permissions()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        })
        // `File::drop` discards a failed `close(2)`, so a deferred write or
        // commit error would be lost and a partial temp file renamed into
        // place. `sync_all` commits the bytes and metadata and surfaces that
        // error while the rename can still be skipped.
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| std::fs::rename(&temp, &destination));
    if let Err(error) = result {
        remove_abandoned_temp(&temp);
        return Err(error);
    }
    Ok(())
}

/// Removes the temp file left by a failed atomic write. The primary error
/// is already on its way to the caller; a cleanup failure is logged once
/// and never replaces it.
fn remove_abandoned_temp(temp: &Path) {
    match std::fs::remove_file(temp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            target: "slugaudit::project",
            path_id = %crate::util::redacted_path(temp),
            %error,
            "failed to remove an abandoned .gitignore temp file"
        ),
    }
}

/// True when `contents` already holds a rule that ignores the activation
/// directory.
///
/// The rule recognised is exact-path, not glob: a line counts when, with
/// trailing whitespace removed (git ignores it unless escaped), one leading
/// `/` removed (at the project root it anchors to the same directory) and
/// trailing `/`s removed, it equals `.planning/slugaudit`. So
/// `.planning/slugaudit/`, `.planning/slugaudit`, `/.planning/slugaudit/` and
/// `/.planning/slugaudit` are all present. `#` comments and `!` negations are
/// not rules that ignore anything. Broader patterns (`.planning/`,
/// `**/slugaudit/`, a matching rule in a parent directory) are not
/// recognised; the duplicate rule that results is harmless, since gitignore
/// permits repeats and resolves them last-match-wins.
fn ignores_activation_dir(contents: &[u8]) -> bool {
    String::from_utf8_lossy(contents).lines().any(|line| {
        let rule = line.trim_end();
        if rule.starts_with('#') || rule.starts_with('!') {
            return false;
        }
        rule.strip_prefix('/').unwrap_or(rule).trim_end_matches('/') == ACTIVATION_RELATIVE_DIR
    })
}

#[cfg(test)]
#[path = "gitignore_tests.rs"]
mod tests;
