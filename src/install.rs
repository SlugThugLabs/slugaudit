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

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("could not locate the running binary: {0}")]
    CurrentExe(std::io::Error),
    #[error("could not determine an install location: neither SLUGTHUG_HOME nor HOME is set")]
    NoHome,
    #[error("could not create {path}: {inner}")]
    Mkdir { path: String, inner: std::io::Error },
    #[error("could not copy the binary to {path}: {inner}")]
    Copy { path: String, inner: std::io::Error },
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

    let temporary = bin_dir.join(format!(
        ".slugaudit.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));
    std::fs::copy(&source, &temporary).map_err(|inner| InstallError::Copy {
        path: temporary.display().to_string(),
        inner,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&temporary)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&temporary, permissions)?;
    }
    std::fs::rename(&temporary, &target).map_err(|inner| InstallError::Copy {
        path: target.display().to_string(),
        inner,
    })?;

    println!("Installed slugaudit to {}", target.display());
    // `install` is human-facing; `connect` follows naturally after PATH has
    // it, so offer to add the dir rather than forcing the user to type an
    // export by hand. No-op when stdin isn't a terminal or already on PATH.
    if !on_path(&bin_dir)
        && interactive_prompt(&format!("Add {} to your PATH? [y/N] ", bin_dir.display()))
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
    let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|entry| std::fs::canonicalize(&entry).unwrap_or(entry) == canonical)
}

/// Reads a single `y`/`n` answer from stdin, defaulting to no. Non-
/// interactive (piped) stdin returns `false` immediately.
fn interactive_prompt(msg: &str) -> bool {
    #[cfg(test)]
    {
        let _ = msg;
        false
    }
    #[cfg(not(test))]
    {
        use std::io::{BufRead as _, IsTerminal as _, Write as _};
        if !std::io::stdin().is_terminal() {
            return false;
        }
        print!("{msg}");
        if std::io::stdout().flush().is_err() {
            return false;
        }
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

/// Appends an `export PATH="<dir>:$PATH"` line to the user's shell config
/// if that exact line isn't already there (idempotent). Chooses the config
/// file from `$SHELL` with a bash/zsh/profile fallback.
fn add_to_path(bin_dir: &Path) -> Result<(), InstallError> {
    let (config_path, line) = path_config(bin_dir)?;
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
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
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_owned());
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
