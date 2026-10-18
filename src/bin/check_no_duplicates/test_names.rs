//! `#[test]` function names defined in more than one file under `src/` or `tests/`.

use std::collections::HashMap;
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
        walk_rs(&src, &mut out, /* skip_substr= */ None, failures);
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

fn walk_rs(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    skip_substr: Option<&str>,
    failures: &mut Vec<String>,
) {
    for entry in read_dir_entries(dir, failures) {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            // ignore_rules.rs and ignore_rules_tests.rs etc. we keep;
            // there's no per-test file we need to single out by name.
            // Caller-supplied skip-substr is honored if any.
            if let Some(skip) = skip_substr
                && name.contains(skip)
            {
                continue;
            }
            walk_rs(&path, out, skip_substr, failures);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

pub(super) fn test_names_by_file(
    paths: &[PathBuf],
    failures: &mut Vec<String>,
) -> HashMap<String, Vec<PathBuf>> {
    let mut names: HashMap<String, Vec<PathBuf>> = HashMap::new();
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
                    .push(path.clone());
                continue;
            }
            // Block form: subsequent lines may include blank lines,
            // comments, and further attributes before the fn item. Skip
            // those, but stop at the first line that resembles a fn
            // declaration.
            let mut cursor = idx + 1;
            while cursor < lines.len() {
                let trimmed = lines[cursor].trim();
                if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#[") {
                    cursor += 1;
                    continue;
                }
                if let Some(name) = block_line_fn_name(trimmed) {
                    names
                        .entry(name.to_string())
                        .or_default()
                        .push(path.clone());
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
/// Trims leading whitespace before scanning (matches the prior Python
/// regex's `^\s*` anchor) and treats the first `]` after `#[` as the
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

fn same_line_fn_name(after_attr: &str) -> Option<&str> {
    let trimmed = after_attr.trim_start();
    let name = if let Some(after) = trimmed.strip_prefix("pub") {
        after.trim_start().strip_prefix("fn")?.trim_start()
    } else if let Some(after) = trimmed.strip_prefix("async") {
        after.trim_start().strip_prefix("fn")?.trim_start()
    } else {
        trimmed.strip_prefix("fn")?.trim_start()
    };
    let end = name
        .find(|c: char| !is_path_seg_char(c))
        .unwrap_or(name.len());
    if end == 0 {
        return None;
    }
    Some(&name[..end])
}

fn block_line_fn_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    same_line_fn_name(trimmed)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
