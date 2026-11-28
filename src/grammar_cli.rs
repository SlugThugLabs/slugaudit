//! CLI subcommand `slugaudit grammar` for inspecting locally installed
//! Tree-sitter grammars and explicitly downloading user-approved grammars from
//! GitHub.

use crate::cli::out;
use crate::parse::{PACK_ALIASES, PACK_VERSION, is_prose_language_name, language_available};
use thiserror::Error;

/// Action requested for `slugaudit grammar`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarAction {
    /// List statically bundled and locally cached grammars.
    List,
    /// Download and cache a non-Rust grammar from GitHub after user confirmation.
    Install {
        /// Canonical language name (e.g. `python`).
        language: String,
        /// Skip the interactive `[y/N]` prompt (`--yes` / `-y`).
        yes: bool,
    },
}

/// Errors returned by `slugaudit grammar`.
#[derive(Debug, Error)]
pub enum GrammarCliError {
    // Spelled out rather than `transparent`: transparent forwards `source()`
    // through the io::Error, which hides the error itself from
    // `cli::is_broken_pipe`'s chain walk.
    /// Writing CLI output failed. A closed pipe is treated as a clean exit.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The requested language name is invalid, an alias, prose, or unknown.
    #[error("{0}")]
    InvalidLanguage(String),
    /// The user declined the download or ran non-interactively without `--yes`.
    #[error("grammar download declined for `{0}`; pass `--yes` to confirm non-interactively")]
    Declined(String),
    /// Downloading or loading the grammar failed.
    #[error("failed to install grammar `{language}`: {source}")]
    InstallFailed {
        /// Language that failed to install.
        language: String,
        /// Underlying pack error.
        #[source]
        source: tree_sitter_language_pack::Error,
    },
}

/// Executes `slugaudit grammar <list|install>`.
///
/// # Errors
///
/// Returns [`GrammarCliError`] on invalid language input, declined confirmation,
/// download/load failure, or stdout write error.
pub fn run_grammar(action: GrammarAction) -> Result<(), GrammarCliError> {
    match action {
        GrammarAction::List => list_grammars(),
        GrammarAction::Install { language, yes } => install_grammar(&language, yes),
    }
}

fn list_grammars() -> Result<(), GrammarCliError> {
    let mut installed = tree_sitter_language_pack::available_languages()
        .into_iter()
        .filter(|lang| tree_sitter_language_pack::has_parser(lang))
        .collect::<Vec<_>>();
    installed.sort_unstable();
    installed.dedup();
    let mut text = format!("Installed Tree-sitter grammars (pack v{PACK_VERSION}):\n");
    for lang in installed {
        let tag = if lang == "rust" { " (bundled)" } else { "" };
        text.push_str("  ");
        text.push_str(&lang);
        text.push_str(tag);
        text.push('\n');
    }
    out(format_args!("{text}"))?;
    Ok(())
}

fn validate_install_target(raw: &str) -> Result<String, GrammarCliError> {
    let lang = raw.trim();
    if lang.is_empty() {
        return Err(GrammarCliError::InvalidLanguage(
            "grammar language must not be empty".to_owned(),
        ));
    }
    if let Some((_, canonical)) = PACK_ALIASES.iter().find(|(alias, _)| *alias == lang) {
        return Err(GrammarCliError::InvalidLanguage(format!(
            "language `{lang}` is an alias; use `{canonical}`"
        )));
    }
    if is_prose_language_name(lang) {
        return Err(GrammarCliError::InvalidLanguage(format!(
            "language `{lang}` is prose and has no Tree-sitter grammar"
        )));
    }
    if !language_available(lang) {
        return Err(GrammarCliError::InvalidLanguage(format!(
            "unknown language `{lang}`"
        )));
    }
    Ok(lang.to_owned())
}

fn install_grammar(raw: &str, yes: bool) -> Result<(), GrammarCliError> {
    let lang = validate_install_target(raw)?;
    if tree_sitter_language_pack::has_parser(&lang) {
        out(format_args!(
            "Grammar `{lang}` is already installed locally.\n"
        ))?;
        return Ok(());
    }
    let source = format!(
        "https://github.com/xberg-io/tree-sitter-language-pack/releases/download/v{PACK_VERSION}/"
    );
    if !yes {
        let prompt =
            format!("Download and load native Tree-sitter grammar `{lang}` from {source}? [y/N] ");
        if !crate::prompt::confirm(&prompt) {
            return Err(GrammarCliError::Declined(lang));
        }
    }
    tree_sitter_language_pack::get_language(&lang).map_err(|source| {
        GrammarCliError::InstallFailed {
            language: lang.clone(),
            source,
        }
    })?;
    out(format_args!("Installed grammar `{lang}`.\n"))?;
    Ok(())
}

#[cfg(test)]
#[path = "grammar_cli_tests.rs"]
mod tests;
