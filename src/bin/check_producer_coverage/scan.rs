//! Source scanning for the producer-coverage gate: the enum's declared
//! variants, and the production sites that construct them.
//!
//! Nothing here is a hand-kept list of variants or producers — both come out
//! of the source text, so a variant added to an enum (or a producer deleted)
//! changes the gate's answer with no second place to update.

use std::fs;
use std::path::Path;

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
            .unwrap_or(&path)
            .display()
            .to_string();
        sources.push((relative, production_code(&text)));
    }
    Ok(())
}

/// Files the repo's test-module conventions declare: a sibling suite
/// (`*_tests.rs`), the `tests.rs` shorthand, or a shared test fixture
/// (`test_support.rs`, `proptest.rs`). Mirrors `check_source_limits`'s
/// classifier, extended with the fixture names `AGENTS.md` documents.
fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.ends_with("_tests.rs")
                || name == "tests.rs"
                || name == "test_support.rs"
                || name == "proptest.rs"
        })
}

/// Keeps only the production code: `#[cfg(test)]`-attributed items are
/// removed so a test fixture cannot be counted as a producer, and `//` line
/// comments are blanked so a variant's name in a doc comment or a trailing
/// comment cannot be one either.
///
/// Handles the two cfg shapes the repo uses: a single-line item
/// (`#[cfg(test)] use …;`, `#[cfg(test)] mod tests;`) and an item with a
/// braced body (an inline `mod tests { … }`, or a cfg-gated helper `fn`).
/// Brace depth is counted on code only; a brace inside a test string literal
/// could confuse the depth, which at worst over-strips — the gate then
/// reports a variant as uncovered rather than silently accepting a test
/// fixture as a producer.
pub(super) fn production_code(source: &str) -> String {
    let mut kept = String::with_capacity(source.len());
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with("#[cfg(test)]") {
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
                        depth += 1;
                        saw_brace = true;
                    }
                    '}' => depth -= 1,
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
            let Some(next) = lines.next() else {
                break;
            };
            current = next;
        }
        kept.push('\n');
    }
    kept
}

fn code_before_line_comment(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

/// The variant identifiers declared by `pub enum <name>` — parsed from the
/// declaration itself, never a hand-kept copy. `None` when the declaration is
/// absent, which the gate treats as "cannot compute a verdict" rather than as
/// "no variants".
pub(super) fn enum_variants(source: &str, name: &str) -> Option<Vec<String>> {
    let header = format!("pub enum {name}");
    let header_start = source.find(&header)?;
    let open = source[header_start..].find('{')? + header_start;
    let mut variants = Vec::new();
    // Bracket nesting inside a variant payload: only a line that begins at
    // depth 0 declares a variant, and only a `}` at depth 0 ends the enum.
    let mut depth: usize = 0;
    for line in source[open + 1..].lines() {
        let trimmed = line.trim();
        let code = code_before_line_comment(trimmed);
        let starts_at_depth = depth;
        for character in code.chars() {
            match character {
                '{' | '(' | '[' => depth += 1,
                '}' | ')' | ']' => match depth.checked_sub(1) {
                    Some(next) => depth = next,
                    // A closer at depth 0 is the enum's own closing brace.
                    None => return Some(variants),
                },
                _ => {}
            }
        }
        if starts_at_depth > 0 {
            continue;
        }
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("#!")
        {
            continue;
        }
        // A variant is the leading identifier of its line; a payload
        // (`LoadFailed { … }`) or discriminant never changes which variant
        // the line declares.
        let head = trimmed
            .split(['{', '(', '=', ','])
            .next()
            .unwrap_or(trimmed)
            .trim();
        if is_identifier(head) {
            variants.push(head.to_owned());
        }
    }
    Some(variants)
}

/// The variants of `name` constructed in `source` at a *value position* — a
/// struct-literal field (`kind: EvidenceKind::X`), a `let`/assignment right
/// hand side, or a block tail — and never a match arm, a pattern, or a
/// comparison.
///
/// The distinction is load-bearing: the SQL mirror (`kind_text`,
/// `as_sql_text`) enumerates every variant as a match arm, so counting any
/// occurrence would make this gate pass for a variant with no producer.
pub(super) fn producer_variants(source: &str, name: &str) -> Vec<String> {
    let needle = format!("{name}::");
    let mut produced = Vec::new();
    let mut from = 0;
    while let Some(offset) = source[from..].find(&needle) {
        let start = from + offset;
        let after_qualifier = start + needle.len();
        let variant: String = source[after_qualifier..]
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        from = after_qualifier;
        if variant.is_empty() {
            continue;
        }
        let end = after_qualifier + variant.len();
        if is_value_position(source, start, end) {
            produced.push(variant);
        }
    }
    produced
}

fn is_value_position(source: &str, start: usize, end: usize) -> bool {
    if source[end..].trim_start().starts_with("=>") {
        return false;
    }
    let before = source[..start].trim_end();
    let Some(last) = before.chars().next_back() else {
        return false;
    };
    match last {
        ':' | '{' => true,
        // A lone `=` is an assignment or a struct field's `= value`; `==`,
        // `!=`, `<=`, `>=` are comparisons, not constructions.
        '=' => {
            let preceding = &before[..before.len() - last.len_utf8()];
            !matches!(preceding.chars().next_back(), Some('=' | '!' | '<' | '>'))
        }
        _ => false,
    }
}

fn is_identifier(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}
