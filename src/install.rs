//! Stable binary installation and executable-path discovery.
#![allow(clippy::print_stdout)]

use std::path::PathBuf;
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

/// Installs a complete executable and publishes it with an atomic rename.
pub fn run_install() -> Result<(), InstallError> {
    let source = running_binary().map_err(InstallError::CurrentExe)?;
    let bin_dir = slugthug_home().ok_or(InstallError::NoHome)?.join("bin");
    let target = bin_dir.join("slugaudit-mcp");
    std::fs::create_dir_all(&bin_dir).map_err(|inner| InstallError::Mkdir {
        path: bin_dir.display().to_string(),
        inner,
    })?;

    let temporary = bin_dir.join(format!(
        ".slugaudit-mcp.{}.{}.tmp",
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

    println!("Installed slugaudit-mcp to {}", target.display());
    println!(
        "Add {} to your PATH, then run: slugaudit-mcp connect",
        bin_dir.display()
    );
    Ok(())
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
