//! The PATH concern of `install`: choose the shell's config file, quote the
//! install directory for that shell, and append the line exactly once.

use super::InstallError;
use std::path::{Path, PathBuf};

/// Appends an `export PATH="<dir>:$PATH"` line to the user's shell config
/// if that exact line isn't already there (idempotent). Chooses the config
/// file from `$SHELL` (bash, zsh or fish); an unset or unrecognized shell is
/// refused with the line to add by hand rather than guessed at.
///
/// The write is append-only plus `fsync`: it never rewrites existing bytes, so
/// a crash cannot truncate the user's rc file, and its permissions and any
/// symlink (dotfile managers) are preserved.
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
    let mut addition = String::new();
    if !existing.ends_with('\n') && !existing.is_empty() {
        addition.push('\n');
    }
    addition.push_str(&line);
    addition.push('\n');
    append_durably(&config_path, &addition).map_err(InstallError::Io)?;
    crate::cli::err(format_args!(
        "Added SlugAudit to your PATH in {}\n",
        config_path.display()
    ))?;
    Ok(())
}

/// Appends `text` to `path` (creating it) and syncs it to disk.
fn append_durably(path: &Path, text: &str) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Returns the shell config file to edit and the exact `PATH` line to
/// append for the current `$SHELL`. Pure so it can be unit-tested.
fn path_config(bin_dir: &Path) -> Result<(PathBuf, String), InstallError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(InstallError::NoHome)?;
    // An unset, non-UTF-8 or unrecognized $SHELL is refused rather than
    // guessed to be bash: editing the wrong rc file is worse than asking.
    let shell = match std::env::var("SHELL") {
        Ok(shell) => shell,
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(std::env::VarError::NotUnicode(_)) => return Err(InstallError::ShellNotUnicode),
    };
    let name = Path::new(&shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
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
        "bash" => Ok((
            home.join(".bashrc"),
            format!("export PATH=\"{}:$PATH\"", double_quoted(&directory)),
        )),
        _ => Err(InstallError::UnknownShell {
            shell,
            line: format!("export PATH=\"{}:$PATH\"", double_quoted(&directory)),
        }),
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
