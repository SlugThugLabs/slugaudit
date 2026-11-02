//! `#[test]` function names defined in more than one file under `src/` or `tests/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Read a directory, recording an unreadable directory or entry as a gate
/// failure: a subtree that cannot be scanned must not let the gate print PASS.
fn read_dir_entries(dir: &Path, failures: &mut Vec<String>) -> Vec<std::fs::DirEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            failures.push(format!("cannot read directory {}: {err}", dir.display()));
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => out.push(entry),
            Err(err) => failures.push(format!(
                "cannot read an entry of directory {}: {err}",
                dir.display()
            )),
        }
    }
    out
}

pub(super) fn collect_test_files(root: &Path, failures: &mut Vec<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let src = root.join("src");
    if src.is_dir() {
        walk_rs(&src, &mut out, failures);
    }
    // Integration tests under tests/*.rs (not tests/fixtures/...).
    let tests = root.join("tests");
    if tests.is_dir() {
        for entry in read_dir_entries(&tests, failures) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") && path.is_file() {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>, failures: &mut Vec<String>) {
    for entry in read_dir_entries(dir, failures) {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, out, failures);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Every `#[test]` function name with the set of files defining it. A name
/// defined twice in one file is still one file: the gate is about names
/// shared across files.
pub(super) fn test_names_by_file(
    paths: &[PathBuf],
    failures: &mut Vec<String>,
) -> BTreeMap<String, BTreeSet<PathBuf>> {
    let mut names: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
    for path in paths {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(err) => {
                failures.push(format!("cannot read {}: {err}", path.display()));
                continue;
            }
        };
        let lines: Vec<&str> = source.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let line = line.trim_start();
            let Some(after_attr) = strip_test_attr(line) else {
                continue;
            };
            // Same-line form: `#[test] fn foo() { ... }` (possibly after
            // whitespace). The attribute regex deliberately matches
            // only `#[test]`-style attributes (optionally path-qualified
            // like `#[tokio::test]`), never `#[cfg(test)]`, so a
            // cfg-gated helper `fn` is not mistaken for a test.
            if let Some(name) = same_line_fn_name(after_attr) {
                names
                    .entry(name.to_string())
                    .or_default()
                    .insert(path.clone());
                continue;
            }
            // Block form: subsequent lines may include blank lines,
            // comments, and further attributes before the fn item. Skip
            // those, but stop at the first line that resembles a fn
            // declaration.
            for following in lines.iter().skip(idx + 1) {
                let trimmed = following.trim();
                if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#[") {
                    continue;
                }
                if let Some(name) = block_line_fn_name(trimmed) {
                    names
                        .entry(name.to_string())
                        .or_default()
                        .insert(path.clone());
                }
                break;
            }
        }
    }
    names
}

/// Returns the trailing text after a `#[test]`-style attribute. Matches
/// `#[test]`, `#[tokio::test]`, `#[some::path::test]` exactly — never
/// `#[cfg(test)]` (the cfg-gated helper) and never `#[test_case]`.
/// Trims leading whitespace before scanning and treats the first `]` after `#[` as the
/// attribute closer rather than requiring the `]` at the very end.
fn strip_test_attr(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("#[")?;
    let close_idx = rest.find(']')?;
    let attr = &rest[..close_idx];
    let after = &rest[close_idx + 1..];
    if attr == "test" {
        return Some(after);
    }
    if let Some(prefix) = attr.strip_suffix("::test") {
        if prefix.is_empty() {
            return None;
        }
        if prefix
            .split("::")
            .all(|seg| !seg.is_empty() && seg.chars().all(is_path_seg_char))
        {
            return Some(after);
        }
    }
    None
}

fn is_path_seg_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The name of the `fn` that `after_attr` starts, skipping a visibility
/// (`pub`, `pub(crate)`, `pub(in path)`) and the `const` / `async` / `unsafe`
/// qualifiers in any order.
fn same_line_fn_name(after_attr: &str) -> Option<&str> {
    let mut rest = after_attr.trim_start();
    if let Some(after_pub) = strip_word(rest, "pub") {
        rest = after_pub.trim_start();
        if let Some(restriction) = rest.strip_prefix('(') {
            rest = restriction.split_once(')')?.1.trim_start();
        }
    }
    let name = loop {
        if let Some(after_fn) = strip_word(rest, "fn") {
            break after_fn.trim_start();
        }
        let qualified = ["const", "async", "unsafe"]
            .iter()
            .find_map(|qualifier| strip_word(rest, qualifier))?;
        rest = qualified.trim_start();
    };
    let end = match name.find(|c: char| !is_path_seg_char(c)) {
        Some(index) => index,
        None => name.len(),
    };
    if end == 0 {
        return None;
    }
    Some(&name[..end])
}

/// `text` after the leading keyword `word`, when `word` is a whole word
/// (`fnord` does not start with the keyword `fn`).
fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let after = text.strip_prefix(word)?;
    after
        .chars()
        .next()
        .is_none_or(|next| !is_path_seg_char(next))
        .then_some(after)
}

fn block_line_fn_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    same_line_fn_name(trimmed)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
