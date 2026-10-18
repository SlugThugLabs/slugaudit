//! Stable binary installation and executable-path discovery.
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};
use thiserror::Error;

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
    #[error("could not locate the running binary: {0}")]
    CurrentExe(std::io::Error),
    #[error("could not determine an install location: neither SLUGTHUG_HOME nor HOME is set")]
    NoHome,
    #[error("could not create {path}: {inner}")]
    Mkdir { path: String, inner: std::io::Error },
    #[error("could not copy the binary to {path}: {inner}")]
    Copy { path: String, inner: std::io::Error },
    #[error("could not read the system clock to name the staged binary: {0}")]
    Clock(std::time::SystemTimeError),
    #[error("$SHELL is not valid UTF-8, so the shell config to edit cannot be chosen")]
    ShellNotUnicode,
    #[error(transparent)]
    Place(#[from] crate::install_place::PlaceError),
    #[error(transparent)]
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

/// Installs a complete executable and publishes it with an atomic rename,
/// then offers to put `bin_dir` on `PATH` (interactively). Adding to PATH is
/// always opt-in: an unattended install (piped stdin / CI) keeps the manual
/// hint instead of editing a shell config it can't confirm.
pub fn run_install() -> Result<(), InstallError> {
    let source = running_binary().map_err(InstallError::CurrentExe)?;
    let bin_dir = slugaudit_dir().ok_or(InstallError::NoHome)?;
    let target = bin_dir.join("slugaudit");
    std::fs::create_dir_all(&bin_dir).map_err(|inner| InstallError::Mkdir {
        path: bin_dir.display().to_string(),
        inner,
    })?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(InstallError::Clock)?
        .as_nanos();
    let temporary = bin_dir.join(format!(".slugaudit.{}.{stamp}.tmp", std::process::id()));
    std::fs::copy(&source, &temporary).map_err(|inner| {
        crate::install_place::remove_leftover(&temporary, "could not remove staged binary");
        InstallError::Copy {
            path: temporary.display().to_string(),
            inner,
        }
    })?;
    crate::install_place::place_staged_binary(&temporary, &target)?;

    println!("Installed slugaudit to {}", target.display());
    // `install` is human-facing; `connect` follows naturally after PATH has
    // it, so offer to add the dir rather than forcing the user to type an
    // export by hand. No-op when stdin isn't a terminal or already on PATH.
    if !on_path(&bin_dir)
        && crate::prompt::confirm(&format!("Add {} to your PATH? [y/N] ", bin_dir.display()))
    {
        match add_to_path(&bin_dir) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("could not update shell config: {error}");
                println!(
                    "Add {} to your PATH, then run: slugaudit connect",
                    bin_dir.display()
                );
            }
        }
    } else if !on_path(&bin_dir) {
        println!(
            "Add {} to your PATH, then run: slugaudit connect",
            bin_dir.display()
        );
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

/// Appends an `export PATH="<dir>:$PATH"` line to the user's shell config
/// if that exact line isn't already there (idempotent). Chooses the config
/// file from `$SHELL` with a bash/zsh/profile fallback.
fn add_to_path(bin_dir: &Path) -> Result<(), InstallError> {
    let (config_path, line) = path_config(bin_dir)?;
    let existing =
        match crate::util::read_file_capped(&config_path, crate::util::MAX_FOREIGN_TEXT_BYTES) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(InstallError::Io(error)),
        };
    if existing.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    let mut contents = existing;
    if !contents.ends_with('\n') && !contents.is_empty() {
        contents.push('\n');
    }
    contents.push_str(&line);
    contents.push('\n');
    std::fs::write(&config_path, contents).map_err(InstallError::Io)?;
    eprintln!("Added SlugAudit to your PATH in {}", config_path.display());
    Ok(())
}

/// Returns the shell config file to edit and the exact `PATH` line to
/// append for the current `$SHELL`. Pure so it can be unit-tested.
fn path_config(bin_dir: &Path) -> Result<(PathBuf, String), InstallError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(InstallError::NoHome)?;
    // An unset $SHELL defaults to bash; a set but non-UTF-8 one is refused
    // rather than silently treated as bash.
    let shell = match std::env::var("SHELL") {
        Ok(shell) => shell,
        Err(std::env::VarError::NotPresent) => "/bin/bash".to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => return Err(InstallError::ShellNotUnicode),
    };
    let name = Path::new(&shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("bash");
    let quoted = bin_dir.display().to_string();
    match name {
        "fish" => Ok((
            home.join(".config/fish/config.fish"),
            format!("fish_add_path {quoted}"),
        )),
        "zsh" => Ok((
            home.join(".zshrc"),
            format!("export PATH=\"{quoted}:$PATH\""),
        )),
        // bash and anything unknown default to ~/.bashrc.
        _ => Ok((
            home.join(".bashrc"),
            format!("export PATH=\"{quoted}:$PATH\""),
        )),
    }
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
