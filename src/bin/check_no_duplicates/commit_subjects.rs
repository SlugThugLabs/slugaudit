//! Duplicate commit subjects in the branch's own history.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

/// A git failure is a FAILURE, not a skip. Every branch below used to print
/// "skipping commit-subject check" and return empty, which `main` then counted
/// as a clean run and printed `no-duplicates: PASS` for — half the gate
/// silently unrun on any checkout where git did not answer, which is the one
/// situation where the check cannot be trusted to have happened.
pub(super) fn duplicate_commit_subjects(
    root: &Path,
    failures: &mut Vec<String>,
) -> Vec<(String, usize)> {
    let range = std::env::var("SLUGAUDIT_CHECK_COMMITS_BASE")
        .map(|b| format!("{b}..HEAD"))
        .unwrap_or_else(|_| {
            let ok = Command::new("git")
                .args(["rev-parse", "--verify", "origin/main"])
                .current_dir(root)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                "origin/main..HEAD".to_owned()
            } else {
                "HEAD".to_owned()
            }
        });
    let output = match Command::new("git")
        .args(["log", &range, "--format=%s"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(proc) => proc,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            failures.push("git not found; cannot check commit subjects".to_owned());
            return Vec::new();
        }
        Err(err) => {
            failures.push(format!(
                "git invocation failed: {err}; cannot check commit subjects"
            ));
            return Vec::new();
        }
    };
    if !output.status.success() {
        failures.push(format!(
            "git log exited with {}; cannot check commit subjects",
            output.status
        ));
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
