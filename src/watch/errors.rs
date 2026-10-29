//! Static, path-free categories for watcher failures.
//!
//! `notify::Error` and `ignore::Error` both format paths into their
//! `Display` output, so a watcher failure is never logged by formatting the
//! error itself: these helpers reduce it to a static category (and, for
//! `ignore`, an optional path the caller redacts) so the log line carries no
//! raw project path.

use std::path::Path;

/// The `notify::Error` variant as a static, path-free category.
/// `notify::Error`'s `Display` appends `about [paths]` when the error carries
/// paths (notify 8.2.0, `error.rs:120-124`), so the error itself is never
/// formatted into a log line.
pub(super) fn notify_error_kind(error: &notify::Error) -> &'static str {
    match error.kind {
        notify::ErrorKind::Generic(_) => "generic",
        notify::ErrorKind::Io(_) => "io",
        notify::ErrorKind::PathNotFound => "path_not_found",
        notify::ErrorKind::WatchNotFound => "watch_not_found",
        notify::ErrorKind::InvalidConfig(_) => "invalid_config",
        notify::ErrorKind::MaxFilesWatch => "max_files_watch",
    }
}

/// Logs a failed watcher operation with the redacted identity of its subject
/// (`root_id`/`dir_id`), the error category, and the number of paths the
/// error carried. Shared by the manager and the scope so every watcher
/// failure has one shape; the error's `Display` is logged as the cause.
pub(super) fn log_notify_failure(op: &'static str, root_id: &str, error: &notify::Error) {
    tracing::warn!(operation = %op, root_id = %root_id, error_kind = notify_error_kind(error), path_count = error.paths.len(), error = %error, "filesystem watcher operation failed");
}

/// The `ignore::Error` variant as a static, path-free category. `Display`
/// for `WithPath` is `{path}: {err}` and `Loop` embeds both paths (ignore
/// 0.4.33, `lib.rs:338-340`), so a walker error is never formatted into a
/// log line.
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

/// The path an `ignore::Error` carries, if any, for redacted correlation.
pub(crate) fn ignore_error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            ignore_error_path(err)
        }
        _ => None,
    }
}
