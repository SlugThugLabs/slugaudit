//! Removal of the temp files an interrupted `config.json` update leaves behind.
//!
//! An update writes `config.json.<pid>.<n>.tmp` beside the config, syncs it and
//! renames it over `config.json` (`config_store.rs`). A failed update removes
//! its own temp file, so only a crash or kill between the create and the rename
//! leaves one. Every load of the project config calls [`remove_stale_temps`].
//!
//! "Stale" is decided by the lock, not by age. A writer creates its temp file
//! only while it holds `config.lock`, so a matching file that exists while the
//! lock can be taken without waiting has no live writer and can never be
//! renamed into place. An age rule could not promise that: a stalled disk can
//! hold a live write past any threshold. The lock is only tried when a matching
//! file exists, so a clean directory never gains a `config.lock` from a read.
//!
//! Failing to scan, lock or delete is a documented, deliberate recovery and not
//! a silent fallback: a leftover temp file is read by nothing, so failing every
//! tool call over it would turn litter into an outage. Each such failure is
//! logged with its source error, once per process per project and file, and the
//! file is left for the next load. The project root is logged as `root`; the
//! file name is logged only after it matched the strict temp-file pattern, so it
//! can only be this module's own `pid.counter` format.

use super::config_store::{ConfigWriteError, acquire_lock, config_path, lock_path};
use crate::util::lock_or_recover;
use std::collections::BTreeSet;
use std::fs::File;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// The `<root id>/<what>` pairs already warned about as failures, so a
/// permanent failure is reported once and not on every call.
static REPORTED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// `true` the first time this process sees `(root_label, what)`.
fn first_report(root_label: &str, what: &str) -> bool {
    lock_or_recover(&REPORTED, "stale_config_temp_warnings").insert(format!("{root_label}/{what}"))
}

/// Whether `name` is exactly what `replace_atomically` creates:
/// `config.json.<pid>.<sequence>.tmp`, both numbers ASCII digits.
fn is_temp_name(name: &str) -> bool {
    let Some(middle) = name
        .strip_prefix("config.json.")
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((pid, sequence)) = middle.split_once('.') else {
        return false;
    };
    [pid, sequence].iter().all(|part| {
        !part.is_empty() && part.len() <= 20 && part.bytes().all(|b| b.is_ascii_digit())
    })
}

/// The names in `directory` that match the temp-file pattern, sorted.
fn temp_names(directory: &Path) -> std::io::Result<Vec<String>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let file_name = entry?.file_name();
        if let Some(name) = file_name.to_str().filter(|name| is_temp_name(name)) {
            found.push(name.to_owned());
        }
    }
    found.sort();
    Ok(found)
}

/// Removes every stale `config.json.<pid>.<n>.tmp` in the project's
/// `.planning/slugaudit/`, with one warning naming each. Never fails: see the
/// module docs for why that is a deliberate recovery.
pub(super) fn remove_stale_temps(project_root: &Path) {
    let directory = project_root.join(".planning").join("slugaudit");
    let root_label = project_root.display().to_string();
    let names = match temp_names(&directory) {
        Ok(names) => names,
        // No activation directory: nothing to clean, and the load that
        // follows reports the missing config in its own way.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            if first_report(&root_label, "scan") {
                tracing::warn!(
                    root = %root_label,
                    %error,
                    "could not scan for stale config.json temp files; leaving them"
                );
            }
            return;
        }
    };
    if names.is_empty() {
        return;
    }
    let Some(_held) = take_lock(project_root, &root_label) else {
        return;
    };
    for name in &names {
        remove_one(&directory, &root_label, name);
    }
}

/// Takes `config.lock` without waiting. `None` means the cleanup is skipped:
/// either a writer holds the lock (expected, not a warning) or the lock could
/// not be taken (warned once).
fn take_lock(project_root: &Path, root_label: &str) -> Option<File> {
    let lock = match lock_path(&config_path(project_root)) {
        Ok(lock) => lock,
        Err(error) => {
            report_lock_failure(root_label, &error);
            return None;
        }
    };
    match acquire_lock(&lock, Duration::ZERO) {
        Ok(held) => Some(held),
        Err(ConfigWriteError::Locked { .. }) => {
            tracing::debug!(
                root = %root_label,
                "config.json is being updated; stale temp files are looked at again on a later load"
            );
            None
        }
        Err(error) => {
            report_lock_failure(root_label, &error);
            None
        }
    }
}

/// Warns once that `config.lock` could not be taken. The error's own text
/// carries a raw path, so only its source or a fixed message is logged.
fn report_lock_failure(root_label: &str, error: &ConfigWriteError) {
    if !first_report(root_label, "lock") {
        return;
    }
    match error {
        ConfigWriteError::Io { source, .. } => tracing::warn!(
            root = %root_label,
            error = %source,
            "could not take config.lock to remove stale config.json temp files; leaving them"
        ),
        ConfigWriteError::NotRegularFile { .. } => tracing::warn!(
            root = %root_label,
            "config.lock is a symlink or not a regular file; stale config.json temp files are left in place"
        ),
        // `lock_path` and `acquire_lock` return no other kind.
        _ => tracing::warn!(
            root = %root_label,
            "config.lock could not be taken for an unexpected reason; stale config.json temp files are left in place"
        ),
    }
}

/// Removes one matching entry, never following a link.
fn remove_one(directory: &Path, root_label: &str, name: &str) {
    let path = directory.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => match std::fs::remove_file(&path) {
            Ok(()) => tracing::warn!(
                root = %root_label,
                file = name,
                "removed a stale config.json temp file left by an interrupted update"
            ),
            // Gone already (removed by hand): nothing left to do.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                if first_report(root_label, name) {
                    tracing::warn!(
                        root = %root_label,
                        file = name,
                        %error,
                        "failed to remove a stale config.json temp file; it is left in place"
                    );
                }
            }
        },
        Ok(_) => {
            if first_report(root_label, name) {
                tracing::warn!(
                    root = %root_label,
                    file = name,
                    "a config.json temp-file name is a symlink or not a regular file, so it was not touched"
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            if first_report(root_label, name) {
                tracing::warn!(
                    root = %root_label,
                    file = name,
                    %error,
                    "could not inspect a config.json temp file; it is left in place"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "config_cleanup_tests.rs"]
mod tests;
