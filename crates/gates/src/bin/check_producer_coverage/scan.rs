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
    collect(&src, root, &mut sources)?;
    sources.sort();
    Ok(sources)
}

fn collect(directory: &Path, root: &Path, sources: &mut Vec<Source>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, sources)?;
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

/// Keeps only production code tokens:
/// 1. Blanks `//` line comments, nested `/* ... */` block comments, `"..."`
///    strings, `r#"..."#` raw strings, and character literals via
///    [`strip_non_code`] so variant names inside comments or literals never
///    count as producers (and braces inside them never skew brace depth).
/// 2. Drops items that exist only under `test` (`#[cfg(test)]`,
///    `#[cfg(all(test, unix))]`; see [`is_test_cfg`]).
///
/// # Errors
///
/// Returns a message when the brace depth of a test item overflows.
pub(super) fn production_code(source: &str) -> Result<String, String> {
    let stripped = strip_non_code(source);
    let mut kept = String::with_capacity(stripped.len());
    let mut lines = stripped.lines();
    while let Some(line) = lines.next() {
        if !is_test_cfg(line) {
            kept.push_str(line);
            kept.push('\n');
            continue;
        }
        let mut depth: i64 = 0;
        let mut saw_brace = false;
        let mut current = line;
        loop {
            for character in current.chars() {
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
                current.trim_end().ends_with(';')
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

/// Blanks `//` line comments, nested `/* ... */` block comments, and the
/// interiors of `"..."`, `r#"..."#`, and character literals while preserving
/// newlines and outer quote delimiters.
pub(super) fn strip_non_code(mut rest: &str) -> String {
    let mut out = String::with_capacity(rest.len());
    let mut prev_is_ident = false;
    while !rest.is_empty() {
        if let Some(after_slashes) = rest.strip_prefix("//") {
            match after_slashes.split_once('\n') {
                Some((_, next_line)) => {
                    out.push('\n');
                    rest = next_line;
                }
                None => break,
            }
            prev_is_ident = false;
            continue;
        }
        if let Some(after_open) = rest.strip_prefix("/*") {
            out.push(' ');
            rest = consume_block_comment(after_open, &mut out);
            prev_is_ident = false;
            continue;
        }
        if !prev_is_ident && let Some((hashes, body)) = raw_string_open(rest) {
            out.push('"');
            rest = consume_raw_string(hashes, body, &mut out);
            prev_is_ident = false;
            continue;
        }
        if !prev_is_ident
            && let Some(body) = rest
                .strip_prefix("b\"")
                .or_else(|| rest.strip_prefix("c\""))
        {
            out.push('"');
            rest = consume_quoted_string(body, &mut out);
            prev_is_ident = false;
            continue;
        }
        if let Some(body) = rest.strip_prefix('"') {
            out.push('"');
            rest = consume_quoted_string(body, &mut out);
            prev_is_ident = false;
            continue;
        }
        if !prev_is_ident
            && let Some(after_char) = rest
                .strip_prefix('b')
                .and_then(consume_char_literal)
                .or_else(|| consume_char_literal(rest))
        {
            out.push_str("''");
            rest = after_char;
            prev_is_ident = false;
            continue;
        }
        let mut chars = rest.chars();
        let Some(ch) = chars.next() else { break };
        out.push(ch);
        prev_is_ident = ch.is_ascii_alphanumeric() || ch == '_';
        rest = chars.as_str();
    }
    out
}

/// Consumes a nested `/* ... */` block comment after the opening `/*`, keeping
/// physical line breaks intact.
fn consume_block_comment<'a>(mut rest: &'a str, out: &mut String) -> &'a str {
    let mut nesting: Vec<()> = vec![()];
    while !nesting.is_empty() {
        if rest.is_empty() {
            break;
        }
        if let Some(after_open) = rest.strip_prefix("/*") {
            nesting.push(());
            out.push(' ');
            rest = after_open;
        } else if let Some(after_close) = rest.strip_prefix("*/") {
            nesting.pop();
            out.push(' ');
            rest = after_close;
        } else {
            let mut chars = rest.chars();
            let Some(ch) = chars.next() else { break };
            out.push(if ch == '\n' { '\n' } else { ' ' });
            rest = chars.as_str();
        }
    }
    rest
}

/// Recognizes `r"..."`, `r#"..."#`, `br#"..."#`, and `cr#"..."#` at the start
/// of `rest`, returning `(hash_count, slice_after_opening_quote)`.
fn raw_string_open(rest: &str) -> Option<(usize, &str)> {
    let after_prefix = rest
        .strip_prefix("br")
        .or_else(|| rest.strip_prefix("cr"))
        .or_else(|| rest.strip_prefix('r'))?;
    let hashes = after_prefix.chars().take_while(|c| *c == '#').count();
    let after_hashes = after_prefix.trim_start_matches('#');
    let body = after_hashes.strip_prefix('"')?;
    Some((hashes, body))
}

/// Consumes a raw string body up to its closing `"` + `hashes` `#` characters,
/// replacing interior characters with spaces (preserving `\n`).
fn consume_raw_string<'a>(hashes: usize, mut rest: &'a str, out: &mut String) -> &'a str {
    let mut closer = String::from("\"");
    closer.extend(std::iter::repeat_n('#', hashes));
    while !rest.is_empty() {
        if let Some(after_close) = rest.strip_prefix(&closer) {
            out.push('"');
            return after_close;
        }
        let mut chars = rest.chars();
        let Some(ch) = chars.next() else { break };
        out.push(if ch == '\n' { '\n' } else { ' ' });
        rest = chars.as_str();
    }
    rest
}

/// Consumes a `"..."` string body after the opening `"`, handling `\` escapes
/// and replacing interior characters with spaces (preserving `\n`).
fn consume_quoted_string<'a>(mut rest: &'a str, out: &mut String) -> &'a str {
    while !rest.is_empty() {
        let mut chars = rest.chars();
        let Some(ch) = chars.next() else { break };
        rest = chars.as_str();
        if ch == '\\' {
            out.push(' ');
            let mut escaped_chars = rest.chars();
            if let Some(escaped) = escaped_chars.next() {
                out.push(if escaped == '\n' { '\n' } else { ' ' });
                rest = escaped_chars.as_str();
            }
        } else if ch == '"' {
            out.push('"');
            return rest;
        } else {
            out.push(if ch == '\n' { '\n' } else { ' ' });
        }
    }
    rest
}

/// Consumes a Rust character literal (`'x'`, `'\n'`, `'\x7f'`, `'\u{...}'`),
/// returning `None` for lifetimes (`'a`, `'static`).
fn consume_char_literal(rest: &str) -> Option<&str> {
    let after_open = rest.strip_prefix('\'')?;
    if let Some(mut escaped) = after_open.strip_prefix('\\') {
        // An escaped char literal cannot span a newline and closes at the next `'`.
        let mut chars = escaped.chars();
        let first = chars.next()?;
        if first == '\n' {
            return None;
        }
        escaped = chars.as_str();
        while !escaped.is_empty() {
            let mut step = escaped.chars();
            let ch = step.next()?;
            escaped = step.as_str();
            if ch == '\'' {
                return Some(escaped);
            }
            if ch == '\n' {
                return None;
            }
        }
        return None;
    }
    let mut chars = after_open.chars();
    let plain = chars.next()?;
    if matches!(plain, '\'' | '\\' | '\n') {
        return None;
    }
    chars.as_str().strip_prefix('\'')
}
