//! Stable binary installation and executable-path discovery.

use std::path::{Path, PathBuf};
use thiserror::Error;

mod path;

/// The shared home directory for slug-branded products.
pub fn slugthug_home() -> Option<PathBuf> {
    std::env::var_os("SLUGTHUG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".slugthug"))
        })
}

/// The dedicated directory for SlugAudit (`~/.slugthug/slugaudit`).
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
pub fn config_home() -> Option<PathBuf> {
    std::env::var_os("SLUGTHUG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
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
    /// The system clock could not be read to name the staged binary.
    #[error("could not read the system clock to name the staged binary: {0}")]
    Clock(std::time::SystemTimeError),
    /// `$SHELL` is set but not valid UTF-8, so no shell config can be chosen.
    #[error("$SHELL is not valid UTF-8, so the shell config to edit cannot be chosen")]
    ShellNotUnicode,
    /// Publishing the staged binary (copy, permissions or atomic rename)
    /// failed.
    #[error(transparent)]
    Place(#[from] crate::install_place::PlaceError),
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
        if path.is_absolute() || path.components().count() > 1 {
            Ok(path)
        } else {
            which::which(&path).map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string())
            })
        }
    })
}

/// The name retained for the binary a replacement install or update displaces.
/// Both `slugaudit install` and `slugaudit update` publish it next to the
/// binary they replace, so a revert is a copy in the same directory.
pub const PREVIOUS_BINARY_NAME: &str = "slugaudit.prev";

/// Copies the binary at `target` to `slugaudit.prev` beside it, so the install
/// or update that follows can be undone. Returns `None` when there is nothing
/// to retain (a first install).
///
/// Called **before** the replacement rename. That preserves the existing
/// property that a crash during replacement leaves the current binary intact,
/// and it means the retained copy is a complete file rather than one caught
/// mid-write. The copy keeps the source's permission bits, so reverting needs
/// no separate `chmod`.
pub(crate) fn retain_previous_binary(target: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    if !target.try_exists()? {
        return Ok(None);
    }
    let Some(directory) = target.parent() else {
        return Ok(None);
    };
    let previous = directory.join(PREVIOUS_BINARY_NAME);
    std::fs::copy(target, &previous)?;
    Ok(Some(previous))
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
pub fn run_install() -> Result<(), InstallError> {
    let source = running_binary_path(running_binary())?;
    let bin_dir = slugaudit_dir().ok_or(InstallError::NoHome)?;
    let target = bin_dir.join("slugaudit");
    std::fs::create_dir_all(&bin_dir).map_err(|inner| InstallError::Mkdir {
        path: bin_dir.display().to_string(),
        inner,
    })?;

    let stamp = elapsed_since_epoch(std::time::SystemTime::now())?.as_nanos();
    let temporary = bin_dir.join(format!(".slugaudit.{}.{stamp}.tmp", std::process::id()));
    std::fs::copy(&source, &temporary).map_err(|inner| {
        crate::install_place::remove_leftover(&temporary, "could not remove staged binary");
        InstallError::Copy {
            path: temporary.display().to_string(),
            inner,
        }
    })?;
    crate::install_place::place_staged_binary(&temporary, &target)?;

    crate::cli::out(format_args!(
        "Installed slugaudit to {}\n",
        target.display()
    ))?;
    // `install` is human-facing; `connect` follows naturally after PATH has
    // it, so offer to add the dir rather than forcing the user to type an
    // export by hand. No-op when stdin isn't a terminal or already on PATH.
    if !on_path(&bin_dir)
        && crate::prompt::confirm(&format!("Add {} to your PATH? [y/N] ", bin_dir.display()))
    {
        match path::add_to_path(&bin_dir) {
            Ok(()) => {}
            Err(error) => {
                crate::cli::err(format_args!("could not update shell config: {error}\n"))?;
                crate::cli::out(format_args!(
                    "Add {} to your PATH, then run: slugaudit connect\n",
                    bin_dir.display()
                ))?;
            }
        }
    } else if !on_path(&bin_dir) {
        crate::cli::out(format_args!(
            "Add {} to your PATH, then run: slugaudit connect\n",
            bin_dir.display()
        ))?;
    }
    Ok(())
}

/// True when `dir` is already on `PATH` (as a canonicalized entry).
fn on_path(dir: &Path) -> bool {
    let canonical = crate::util::canonical_or_self(dir);
    // An entry that cannot be canonicalized is compared as written; a missing
    // directory is routine on a PATH and silent, any other failure is logged.
    // A missed match only re-offers the (idempotent) PATH-add prompt.
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).any(|entry| {
        match std::fs::canonicalize(&entry) {
            Ok(resolved) => resolved == canonical,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        path = %crate::util::redacted_path(&entry),
                        error = %error,
                        "could not canonicalize a PATH entry; comparing it as written",
                    );
                }
                entry == canonical
            }
        }
    })
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
