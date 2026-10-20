//! The PATH concern of `install`: choose the shell's config file, quote the
//! install directory for that shell, and append the line exactly once.

use super::InstallError;
use std::path::{Path, PathBuf};

/// Appends an `export PATH="<dir>:$PATH"` line to the user's shell config
/// if that exact line isn't already there (idempotent). Chooses the config
/// file from `$SHELL` with a bash/zsh/profile fallback.
pub(crate) fn add_to_path(bin_dir: &Path) -> Result<(), InstallError> {
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
    crate::cli::err(format_args!(
        "Added SlugAudit to your PATH in {}\n",
        config_path.display()
    ))?;
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
    let directory = bin_dir.display().to_string();
    match name {
        "fish" => Ok((
            home.join(".config/fish/config.fish"),
            format!("fish_add_path {}", fish_word(&directory)),
        )),
        "zsh" => Ok((
            home.join(".zshrc"),
            format!("export PATH=\"{}:$PATH\"", double_quoted(&directory)),
        )),
        // bash and anything unknown default to ~/.bashrc.
        _ => Ok((
            home.join(".bashrc"),
            format!("export PATH=\"{}:$PATH\"", double_quoted(&directory)),
        )),
    }
}

/// Escapes a path for a double-quoted bash/zsh string. An ordinary path is
/// returned unchanged, so a line this tool wrote earlier still matches the
/// idempotency check; `$` (which would expand), a backtick, a quote and a
/// backslash are backslash-escaped.
fn double_quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// A fish word for `fish_add_path`: bare when the path holds no character fish
/// treats specially, otherwise single-quoted. Inside fish single quotes only
/// `\` and `'` need escaping.
fn fish_word(value: &str) -> String {
    let bare = value
        .chars()
        .all(|c| c.is_alphanumeric() || "/._-+:,@%=~".contains(c));
    if bare {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
