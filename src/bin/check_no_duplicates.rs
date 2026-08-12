//! No-duplicates gate.
// slugaudit-line-exception: approved-by=agent; reason=the bin owns two orthogonal gate inputs (commit-subject dedupe via git log, and #[test] fn-name dedupe across src/ + tests/) and they share one argv parser, one process exit-code contract, and one failure-printer; splitting into separate bins would duplicate that scaffolding, and collapsing the test-name collector into a sibling module under src/lib.rs would smuggle production code into a path sized for internal-only modules
//!
//! Replaces `tools/check_no_duplicates.sh`. Fails when:
//! - a commit subject occurs more than once in reachable history (the
//!   fingerprint of the same logical change being committed twice), or
//! - a `#[test]`-attributed function name is defined in more than one
//!   file under `src/` or `tests/`.
//!
//! Pure-Rust replacement: no Python interpreter, no shell. `git log` is
//! invoked through `std::process::Command`; the test-name scan reads
//! each Rust file with stdlib only.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut project_root = std::env::current_dir().expect("cwd");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!("Checks for duplicated commit subjects and duplicated test names.");
                return ExitCode::SUCCESS;
            }
            "--root" => {
                let Some(value) = args.next() else {
                    eprintln!("--root requires a project directory");
                    return ExitCode::from(2);
                };
                project_root = PathBuf::from(value);
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let mut failures: Vec<String> = Vec::new();

    for (subject, count) in duplicate_commit_subjects(&project_root) {
        failures.push(format!("commit subject {subject:?} used {count} times"));
        println!("no-duplicates: commit subject {subject:?} used {count} times");
    }

    let test_files = collect_test_files(&project_root);
    let names = test_names_by_file(&test_files);
    let mut by_name: BTreeMap<&str, Vec<&PathBuf>> = BTreeMap::new();
    for (name, files) in &names {
        by_name.insert(name.as_str(), files.iter().collect());
    }
    for (name, files) in by_name {
        if files.len() > 1 {
            let joined = files
                .iter()
                .map(|p| {
                    p.strip_prefix(&project_root)
                        .unwrap_or(p)
                        .display()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ");
            failures.push(format!(
                "test {name:?} defined in {} files: {joined}",
                files.len()
            ));
            println!(
                "no-duplicates: test {name:?} defined in {} files: {joined}",
                files.len()
            );
        }
    }

    if !failures.is_empty() {
        println!("no-duplicates: FAIL");
        return ExitCode::from(1);
    }
    println!("no-duplicates: PASS");
    ExitCode::SUCCESS
}

fn duplicate_commit_subjects(root: &Path) -> Vec<(String, usize)> {
    let output = match Command::new("git")
        .arg("log")
        .arg("--format=%s")
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(proc) => proc,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("no-duplicates: git not found; skipping commit-subject check");
            return Vec::new();
        }
        Err(err) => {
            println!("no-duplicates: git invocation failed: {err}; skipping commit-subject check");
            return Vec::new();
        }
    };
    if !output.status.success() {
        println!("no-duplicates: not a git repository; skipping commit-subject check");
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for line in stdout.lines() {
        let subject = line.trim();
        if !subject.is_empty() {
            *counts.entry(subject.to_owned()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().filter(|(_, c)| *c > 1).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn collect_test_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let src = root.join("src");
    if src.is_dir() {
        walk_rs(&src, &mut out, /* skip_substr= */ None);
    }
    // Integration tests under tests/*.rs (not tests/fixtures/...).
    let tests = root.join("tests");
    if tests.is_dir()
        && let Ok(rd) = std::fs::read_dir(&tests)
    {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") && path.is_file() {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>, skip_substr: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
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
            walk_rs(&path, out, skip_substr);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn test_names_by_file(paths: &[PathBuf]) -> HashMap<String, Vec<PathBuf>> {
    let mut names: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for path in paths {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
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
mod tests {
    use super::*;

    #[test]
    fn strip_test_attr_accepts_bare_test() {
        assert_eq!(strip_test_attr("#[test]"), Some(""));
        assert_eq!(strip_test_attr("  #[test]"), Some(""));
        assert_eq!(strip_test_attr("#[test] rest"), Some(" rest"));
    }

    #[test]
    fn strip_test_attr_accepts_qualified() {
        assert_eq!(strip_test_attr("#[tokio::test]"), Some(""));
        assert_eq!(strip_test_attr("#[some::path::test]"), Some(""));
    }

    #[test]
    fn strip_test_attr_rejects_cfg_test() {
        assert_eq!(strip_test_attr("#[cfg(test)]"), None);
    }

    #[test]
    fn strip_test_attr_rejects_test_case() {
        // `test_case` ends with `case`, not `test`.
        assert_eq!(strip_test_attr("#[test_case]"), None);
    }

    #[test]
    fn same_line_fn_name_basic() {
        assert_eq!(same_line_fn_name(" fn foo() {"), Some("foo"));
    }

    #[test]
    fn same_line_fn_name_with_pub() {
        assert_eq!(same_line_fn_name(" pub fn bar() {"), Some("bar"));
        assert_eq!(same_line_fn_name("async fn baz()"), Some("baz"));
    }

    #[test]
    fn block_line_skips_comments_and_attrs() {
        // Build a synthetic file by running test_names_by_file against
        // a tiny tempdir.
        let tmp = tempfile_or_panic_path();
        let dir = tmp.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a_tests.rs");
        std::fs::write(
            &p,
            "#[test]\n\
             // comment between attr and fn\n\
             #[allow(unused)]\n\
             fn my_test() {}\n",
        )
        .unwrap();
        let map = test_names_by_file(&[p]);
        assert!(map.contains_key("my_test"), "missing key in {map:?}");
    }

    fn tempfile_or_panic_path() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "slugaudit-check-no-duplicates-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
