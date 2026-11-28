//! Stable binary installation and executable-path discovery.

mod path;
mod place;

pub(crate) use place::{INSTALLED_BINARY_NAME, place_staged_binary, remove_leftover};
pub use place::{PREVIOUS_BINARY_NAME, PlaceError};
use place::{next_staging_seq, stage_copy};

use std::path::{Path, PathBuf};
use thiserror::Error;

/// The value of environment variable `name`, treating an empty or relative
/// value as unset: a relative path would resolve against the current directory
/// and leak CWD-relative entries into shell `PATH` or global MCP configs.
fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn home_from_env() -> Option<PathBuf> {
    if let Some(home) = non_empty_var("HOME") {
        return Some(home);
    }
    #[cfg(windows)]
    {
        non_empty_var("USERPROFILE")
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The shared home directory for slug-branded products. An empty
/// `SLUGTHUG_HOME` or `HOME` counts as unset.
#[must_use]
pub fn slugthug_home() -> Option<PathBuf> {
    non_empty_var("SLUGTHUG_HOME").or_else(|| home_from_env().map(|home| home.join(".slugthug")))
}

/// The dedicated directory for SlugAudit (`~/.slugthug/slugaudit`).
#[must_use]
pub fn slugaudit_dir() -> Option<PathBuf> {
    slugthug_home().map(|home| home.join("slugaudit"))
}

/// The **user's** home directory — the base that a client's MCP configuration
/// path is resolved against.
///
/// Deliberately separate from [`slugthug_home`]. That one is SlugAudit's own
/// `~/.slugthug` directory (binary install, runtime state); an editor or agent
/// reads its MCP config from the user's home instead (e.g. `~/.cursor/mcp.json`).
/// Joining a client config path onto `slugthug_home` writes a file the client
/// never reads — a silent failure, because the write succeeds.
///
/// `SLUGTHUG_CONFIG_HOME` overrides the base for tests and for setups where the
/// agents run under a different home than the process that runs `connect`.
/// An empty variable counts as unset.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    non_empty_var("SLUGTHUG_CONFIG_HOME").or_else(home_from_env)
}

#[derive(Debug, Error)]
/// Failures reported by `slugaudit install`, each carrying the path or cause
/// an operator needs.
pub enum InstallError {
    /// The running executable's path could not be resolved.
    #[error("could not locate the running binary: {0}")]
    CurrentExe(std::io::Error),
    /// Neither `SLUGTHUG_HOME` nor `HOME` is set, so there is no install
    /// directory.
    #[error("could not determine an install location: neither SLUGTHUG_HOME nor HOME is set")]
    NoHome,
    /// `HOME` is unset, so the user's shell startup file cannot be located.
    #[error("could not locate shell startup file: HOME is unset")]
    ShellConfigHomeMissing,
    /// The install directory could not be created.
    #[error("could not create {path}: {inner}")]
    Mkdir {
        /// Directory that could not be created.
        path: String,
        /// Underlying filesystem error.
        inner: std::io::Error,
    },
    /// The binary could not be copied to its staging path.
    #[error("could not copy the binary to {path}: {inner}")]
    Copy {
        /// Staging path that could not be written.
        path: String,
        /// Underlying filesystem error.
        inner: std::io::Error,
    },
    /// The install directory could not be resolved to its canonical path,
    /// so whether it is already on `PATH` cannot be told.
    #[error("could not resolve {path}: {inner}")]
    Canonicalize {
        /// Directory that could not be resolved.
        path: String,
        /// Underlying filesystem error.
        inner: std::io::Error,
    },
    /// The system clock could not be read to name the staged binary.
    #[error("could not read the system clock to name the staged binary: {0}")]
    Clock(std::time::SystemTimeError),
    /// The install directory is not valid UTF-8, so a `PATH` line naming it
    /// exactly cannot be written.
    #[error("the install directory {path} is not valid UTF-8, so it cannot be added to PATH")]
    BinDirNotUnicode {
        /// The directory, lossily displayed.
        path: String,
    },
    /// The install directory contains `\n` or `\r`, so a single-line shell
    /// startup entry cannot represent it safely.
    #[error(
        "the install directory {path:?} contains a newline, so it cannot be added to PATH safely"
    )]
    BinDirContainsNewline {
        /// The directory path containing `\n` or `\r`.
        path: String,
    },
    /// `$SHELL` is set but not valid UTF-8, so no shell config can be chosen.
    #[error("$SHELL is not valid UTF-8, so the shell config to edit cannot be chosen")]
    ShellNotUnicode,
    /// `$SHELL` is unset or names a shell this tool does not know, so no
    /// shell config is edited.
    #[error(
        "$SHELL is unset or unrecognized ({shell:?}), so no shell config was edited; \
         add this line to your shell's startup file: {line}"
    )]
    UnknownShell {
        /// The `$SHELL` value, empty when unset.
        shell: String,
        /// The exact POSIX-shell line to add by hand.
        line: String,
    },
    /// Publishing the staged binary (copy, permissions or atomic rename)
    /// failed.
    #[error(transparent)]
    Place(#[from] PlaceError),
    // Spelled out rather than `transparent`: transparent forwards `source()`
    // through the io::Error, which hides the error itself from
    // `cli::is_broken_pipe`'s chain walk.
    /// A filesystem or stdout write failed. A closed pipe is treated as a
    /// clean exit.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub(crate) fn running_binary() -> Result<PathBuf, std::io::Error> {
    std::env::current_exe().or_else(|current_error| {
        let argument = std::env::args_os().next().ok_or(current_error)?;
        let path = PathBuf::from(argument);
        if path.is_absolute() {
            Ok(path)
        } else if path.components().count() > 1 {
            std::env::current_dir().map(|cwd| cwd.join(path))
        } else {
            which::which(&path).map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string())
            })
        }
    })
}

/// Maps the running-binary lookup onto its install failure. Split from
/// [`run_install`] so the `CurrentExe` arm is testable without faking the
/// process that owns the executable path.
fn running_binary_path(result: Result<PathBuf, std::io::Error>) -> Result<PathBuf, InstallError> {
    result.map_err(InstallError::CurrentExe)
}

/// The time elapsed since the Unix epoch, kept as the typed clock failure so a
/// pre-epoch clock is reported rather than panicked on.
fn elapsed_since_epoch(now: std::time::SystemTime) -> Result<std::time::Duration, InstallError> {
    now.duration_since(std::time::UNIX_EPOCH)
        .map_err(InstallError::Clock)
}

/// Installs a complete executable and publishes it with an atomic rename,
/// then offers to put `bin_dir` on `PATH` (interactively). Adding to PATH is
/// always opt-in: an unattended install (piped stdin / CI) keeps the manual
/// hint instead of editing a shell config it can't confirm.
///
/// # Errors
///
/// Fails when the running binary or the home directory cannot be determined,
/// when the install directory cannot be created, when the binary cannot be
/// copied or placed, or when writing the progress text fails. A failure to
/// edit the shell config is reported on stderr and is not an error.
pub fn run_install() -> Result<(), InstallError> {
    run_install_with(crate::prompt::confirm)
}

/// [`run_install`] with the PATH question's asker injected: `confirm` receives
/// the question and returns whether it was accepted.
fn run_install_with(confirm: impl FnOnce(&str) -> bool) -> Result<(), InstallError> {
    let source = running_binary_path(running_binary())?;
    let bin_dir = slugaudit_dir().ok_or(InstallError::NoHome)?;
    let target = bin_dir.join(INSTALLED_BINARY_NAME);
    std::fs::create_dir_all(&bin_dir).map_err(|inner| InstallError::Mkdir {
        path: bin_dir.display().to_string(),
        inner,
    })?;

    let stamp = elapsed_since_epoch(std::time::SystemTime::now())?.as_nanos();
    let seq = next_staging_seq();
    let temporary = bin_dir.join(format!(
        ".slugaudit.{}.{stamp}.{seq}.tmp",
        std::process::id()
    ));
    stage_copy(&source, &temporary).map_err(|inner| InstallError::Copy {
        path: temporary.display().to_string(),
        inner,
    })?;
    place_staged_binary(&temporary, &target)?;

    crate::cli::out(format_args!(
        "Installed slugaudit to {}\n",
        target.display()
    ))?;
    // `install` is human-facing; `connect` follows naturally after PATH has
    // it, so offer to add the dir rather than forcing the user to type an
    // export by hand. No-op when stdin isn't a terminal or already on PATH.
    let already_on_path = on_path(&bin_dir)?;
    let mut print_manual_hint = !already_on_path;
    if !already_on_path && confirm(&format!("Add {} to your PATH? [y/N] ", bin_dir.display())) {
        match path::add_to_path(&bin_dir) {
            Ok(()) => print_manual_hint = false,
            Err(error) => {
                crate::cli::err(format_args!("could not update shell config: {error}\n"))?;
            }
        }
    }
    if print_manual_hint {
        crate::cli::out(format_args!(
            "Add {} to your PATH, then run: slugaudit connect\n",
            bin_dir.display()
        ))?;
    }
    Ok(())
}

/// True when `dir` is already on `PATH` (as a canonicalized entry).
///
/// # Errors
///
/// Returns [`InstallError::Canonicalize`] when `dir` itself cannot be
/// canonicalized.
fn on_path(dir: &Path) -> Result<bool, InstallError> {
    let canonical = std::fs::canonicalize(dir).map_err(|inner| InstallError::Canonicalize {
        path: dir.display().to_string(),
        inner,
    })?;
    let path_var = match std::env::var_os("PATH") {
        Some(value) => value,
        None => std::ffi::OsString::new(),
    };
    Ok(
        std::env::split_paths(&path_var).any(|entry| match std::fs::canonicalize(&entry) {
            Ok(resolved) => resolved == canonical,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        path = %entry.display(),
                        error = %error,
                        "could not canonicalize a PATH entry; comparing it as written",
                    );
                }
                entry == canonical
            }
        }),
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
