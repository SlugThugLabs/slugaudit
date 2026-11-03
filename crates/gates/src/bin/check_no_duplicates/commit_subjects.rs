//! Duplicate commit subjects in the branch's own history.

use std::env::VarError;
use std::path::Path;
use std::process::{Command, Stdio};

/// A git failure is a FAILURE, not a skip. Returning empty would let `main`
/// count it as a clean run and print `no-duplicates: PASS` — half the gate
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
    let mut subjects: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .collect();
    subjects.sort_unstable();
    subjects
        .chunk_by(|a, b| a == b)
        .filter(|group| group.len() > 1)
        .filter_map(|group| {
            group
                .first()
                .map(|subject| ((*subject).to_owned(), group.len()))
        })
        .collect()
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
        Ok(base) if base.trim().is_empty() => {
            failures.push(
                "SLUGAUDIT_CHECK_COMMITS_BASE is set but empty; cannot choose the commit range"
                    .to_owned(),
            );
            return None;
        }
        Ok(base) => return Some(format!("{base}..HEAD")),
        Err(VarError::NotUnicode(raw)) => {
            failures.push(format!(
                "SLUGAUDIT_CHECK_COMMITS_BASE is not valid UTF-8 ({}); cannot choose the commit range",
                raw.display()
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
    use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
    use std::ffi::OsString;

    #[test]
    fn a_base_that_is_not_utf8_fails_instead_of_widening_the_range() -> TestResult {
        let mut failures = Vec::new();

        let range = commit_range(
            Err(VarError::NotUnicode(OsString::from("x"))),
            Path::new("."),
            &mut failures,
        );

        ensure_eq(&range, &None, "range == None")?;
        ensure(
            failures.iter().any(|f| f.contains("not valid UTF-8")),
            format!("{failures:?}"),
        )?;
        Ok(())
    }

    #[test]
    fn a_set_base_selects_its_own_range() -> TestResult {
        let mut failures = Vec::new();

        let range = commit_range(Ok("abc".to_owned()), Path::new("."), &mut failures);

        ensure_eq(
            &range.as_deref(),
            &Some("abc..HEAD"),
            "range.as_deref() == Some(\"abc..HEAD\")",
        )?;
        ensure(failures.is_empty(), "failures.is_empty()")?;
        Ok(())
    }

    #[test]
    fn an_empty_or_blank_base_fails_instead_of_checking_no_commits() -> TestResult {
        for base in ["", "  ", "\t\n"] {
            let mut failures = Vec::new();

            let range = commit_range(Ok(base.to_owned()), Path::new("."), &mut failures);

            ensure_eq(&range, &None, format!("base {base:?}"))?;
            ensure_eq(
                &failures,
                &["SLUGAUDIT_CHECK_COMMITS_BASE is set but empty; cannot choose the commit range"],
                format!("base {base:?}"),
            )?;
        }
        Ok(())
    }

    #[test]
    fn a_failing_git_log_reports_its_stderr() -> TestResult {
        let dir = tempfile::tempdir().check("tempdir")?;
        let mut failures = Vec::new();

        let subjects = duplicate_commit_subjects(dir.path(), &mut failures);

        ensure(subjects.is_empty(), "subjects.is_empty()")?;
        let message = failures.first().check("git log outside a repo must fail")?;
        ensure(message.contains("stderr: "), message)?;
        ensure(
            !message.ends_with("stderr: )"),
            format!("stderr was dropped: {message}"),
        )?;
        Ok(())
    }
}
