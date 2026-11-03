//! Source scanning for the producer-coverage gate: the enum's declared
//! variants, and the production sites that construct them.
//!
//! Nothing here is a hand-kept list of variants or producers — both come out
//! of the source text, so a variant added to an enum (or a producer deleted)
//! changes the gate's answer with no second place to update.

use std::fs;
use std::path::Path;

use crate::gate_test_file::is_test_file;
use crate::test_cfg::is_test_cfg;

/// A production source file: its path relative to the project root, and its
/// contents.
pub(super) type Source = (String, String);

/// Reads every production Rust source under `<root>/src`, sorted by path.
///
/// Excluded, because a producer must be production code:
/// - `src/bin/**` — the gates themselves. A gate does not produce evidence.
/// - test files by the repo's naming conventions (`*_tests.rs`, `tests.rs`,
///   `test_support.rs`, `proptest.rs`).
/// - `#[cfg(test)]`-attributed items inside the remaining files.
///
/// # Errors
///
/// Returns a message when `src` is missing or a file cannot be read.
pub(super) fn production_sources(root: &Path) -> Result<Vec<Source>, String> {
    let src = root.join("src");
    if !src.is_dir() {
        return Err(format!("{} is not a directory", src.display()));
    }
    let mut sources = Vec::new();
    collect(&src, root, true, &mut sources)?;
    sources.sort();
    Ok(sources)
}

fn collect(
    directory: &Path,
    root: &Path,
    is_src_root: bool,
    sources: &mut Vec<Source>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
        let path = entry.path();
        if path.is_dir() {
            // The gates under `src/bin/` are not production code.
            if is_src_root && path.file_name().is_some_and(|name| name == "bin") {
                continue;
            }
            collect(&path, root, false, sources)?;
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") || is_test_file(&path) {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .map_err(|error| {
                format!(
                    "{} is not under the scan root {}: {error}",
                    path.display(),
                    root.display()
                )
            })?
            .display()
            .to_string();
        let code = production_code(&text)
            .map_err(|error| format!("cannot strip test code from {}: {error}", path.display()))?;
        sources.push((relative, code));
    }
    Ok(())
}

/// Keeps only the production code: items that exist only under `test`
/// (`#[cfg(test)]`, `#[cfg(all(test, unix))]`; see [`is_test_cfg`]) are
/// removed so a test fixture cannot be counted as a producer, and `//` line
/// comments are blanked so a variant's name in a doc comment or a trailing
/// comment cannot be one either.
///
/// Handles the two item shapes the repo uses: a single-line item
/// (`#[cfg(test)] use …;`, `#[cfg(test)] mod tests;`) and an item with a
/// braced body (an inline `mod tests { … }`, or a cfg-gated helper `fn`).
/// Brace depth is counted on code only; a brace inside a test string literal
/// could confuse the depth, which at worst over-strips — the gate then
/// reports a variant as uncovered rather than silently accepting a test
/// fixture as a producer.
///
/// # Errors
///
/// Returns a message when the brace depth of a test item overflows.
pub(super) fn production_code(source: &str) -> Result<String, String> {
    let mut kept = String::with_capacity(source.len());
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if !is_test_cfg(line) {
            kept.push_str(code_before_line_comment(line));
            kept.push('\n');
            continue;
        }
        let mut depth: i64 = 0;
        let mut saw_brace = false;
        let mut current = line;
        loop {
            let code = code_before_line_comment(current);
            for character in code.chars() {
                match character {
                    '{' => {
                        depth = depth
                            .checked_add(1)
                            .ok_or_else(|| "brace depth overflowed".to_owned())?;
                        saw_brace = true;
                    }
                    '}' => {
                        depth = depth
                            .checked_sub(1)
                            .ok_or_else(|| "brace depth underflowed".to_owned())?;
                    }
                    _ => {}
                }
            }
            let ended = if saw_brace {
                depth <= 0
            } else {
                code.trim_end().ends_with(';')
            };
            if ended {
                break;
            }
            let Some(next) = lines.next() else { break };
            current = next;
        }
        kept.push('\n');
    }
    Ok(kept)
}

pub(super) fn code_before_line_comment(line: &str) -> &str {
    match line.split_once("//") {
        Some((code, _)) => code,
        None => line,
    }
}
