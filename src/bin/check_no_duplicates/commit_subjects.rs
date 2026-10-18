//! Duplicate commit subjects in the branch's own history.

use std::collections::HashMap;
use std::env::VarError;
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
    let Some(range) = commit_range(
        std::env::var("SLUGAUDIT_CHECK_COMMITS_BASE"),
        root,
        failures,
    ) else {
        return Vec::new();
    };
    let output = match Command::new("git")
        .args(["log", &range, "--format=%s"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
            "git log {range} exited with {}; cannot check commit subjects (stderr: {})",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
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

/// The commit range to check, or `None` (with a recorded failure) when it
/// cannot be determined. A base the caller set but that cannot be honoured, or
/// a git that cannot be asked, is a failure: silently widening to a different
/// range would check commits the caller did not request.
fn commit_range(
    base: Result<String, VarError>,
    root: &Path,
    failures: &mut Vec<String>,
) -> Option<String> {
    match base {
        Ok(base) => return Some(format!("{base}..HEAD")),
        Err(VarError::NotUnicode(raw)) => {
            failures.push(format!(
                "SLUGAUDIT_CHECK_COMMITS_BASE is not valid UTF-8 ({raw:?}); cannot choose the commit range"
            ));
            return None;
        }
        Err(VarError::NotPresent) => {}
    }
    match Command::new("git")
        .args(["rev-parse", "--verify", "origin/main"])
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Some("origin/main..HEAD".to_owned()),
        // origin/main is simply absent: the whole reachable history is the range.
        Ok(_) => Some("HEAD".to_owned()),
        Err(err) => {
            failures.push(format!(
                "git rev-parse origin/main could not run: {err}; cannot choose the commit range"
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn a_base_that_is_not_utf8_fails_instead_of_widening_the_range() {
        let mut failures = Vec::new();

        let range = commit_range(
            Err(VarError::NotUnicode(OsString::from("x"))),
            Path::new("."),
            &mut failures,
        );

        assert_eq!(range, None);
        assert!(
            failures.iter().any(|f| f.contains("not valid UTF-8")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_set_base_selects_its_own_range() {
        let mut failures = Vec::new();

        let range = commit_range(Ok("abc".to_owned()), Path::new("."), &mut failures);

        assert_eq!(range.as_deref(), Some("abc..HEAD"));
        assert!(failures.is_empty());
    }

    #[test]
    fn a_failing_git_log_reports_its_stderr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut failures = Vec::new();

        let subjects = duplicate_commit_subjects(dir.path(), &mut failures);

        assert!(subjects.is_empty());
        let message = failures.first().expect("git log outside a repo must fail");
        assert!(message.contains("stderr: "), "{message}");
        assert!(
            !message.ends_with("stderr: )"),
            "stderr was dropped: {message}"
        );
    }
}
