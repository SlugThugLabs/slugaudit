//! Duplicate commit subjects in reachable history.
//!
//! Direct pushes to `main` leave `origin/main..HEAD` empty, and checking a
//! pull request's range in isolation misses a subject that duplicates a commit
//! already on `main`. When `SLUGAUDIT_CHECK_COMMITS_BASE` is unset, this
//! scanner checks every commit after [`HISTORICAL_BASELINE_COMMIT`] (or all of
//! `HEAD` in repositories that do not contain that commit) against both each
//! other and every prior commit subject in reachable history. When
//! `SLUGAUDIT_CHECK_COMMITS_BASE` is explicitly set, commits in `base..HEAD`
//! are checked against each other and against every commit reachable from
//! `base`.

use std::collections::BTreeMap;
use std::env::VarError;
use std::path::Path;
use std::process::{Command, Stdio};

/// Historical baseline cut (`2026-10-13`, `c996894` — the second of two early
/// `"style: apply rustfmt changes from CI"` commits that predate strict
/// duplicate-subject enforcement on `main`). Every commit after this SHA is
/// checked against all prior commits AND all post-cut commits, so no new
/// duplicate can ever land via a direct push to `main` or via a pull request.
const HISTORICAL_BASELINE_COMMIT: &str = "c9968944ce51694fe8b8155e3bb6712bed566e56";

/// Which commits are audited for duplicate subjects.
#[derive(Debug, PartialEq, Eq)]
enum CommitScope {
    /// Audit every commit reachable from `HEAD` (`git log HEAD`).
    AllReachable,
    /// Audit every commit in `<base>..HEAD` against each other and against all
    /// commits reachable from `<base>`.
    SinceBase(String),
}

/// A git failure is a FAILURE, not a skip. Returning empty without recording a
/// failure would let `main` print `no-duplicates: PASS` when git did not run.
pub(super) fn duplicate_commit_subjects(
    root: &Path,
    failures: &mut Vec<String>,
) -> Vec<(String, usize)> {
    let Some(scope) = commit_scope(
        std::env::var("SLUGAUDIT_CHECK_COMMITS_BASE"),
        root,
        failures,
    ) else {
        return Vec::new();
    };
    match scope {
        CommitScope::AllReachable => {
            let Some(subjects) = git_subjects(root, "HEAD", failures) else {
                return Vec::new();
            };
            let Some(counts) = count_subjects(&subjects, failures) else {
                return Vec::new();
            };
            counts.into_iter().filter(|(_, count)| *count > 1).collect()
        }
        CommitScope::SinceBase(base) => duplicates_since_base(root, &base, failures),
    }
}

/// Checks every commit in `base..HEAD` against other commits in `base..HEAD`
/// and against all commits already reachable from `base`.
fn duplicates_since_base(
    root: &Path,
    base: &str,
    failures: &mut Vec<String>,
) -> Vec<(String, usize)> {
    let range = format!("{base}..HEAD");
    let Some(prior_subjects) = git_subjects(root, base, failures) else {
        return Vec::new();
    };
    let Some(new_subjects) = git_subjects(root, &range, failures) else {
        return Vec::new();
    };
    let Some(prior_counts) = count_subjects(&prior_subjects, failures) else {
        return Vec::new();
    };
    let Some(new_counts) = count_subjects(&new_subjects, failures) else {
        return Vec::new();
    };
    let mut duplicates = Vec::new();
    for (subject, new_count) in new_counts {
        let prior_count = match prior_counts.get(&subject) {
            Some(count) => *count,
            None => 0,
        };
        let Some(total) = prior_count.checked_add(new_count) else {
            failures.push(format!("commit subject count overflowed for {subject:?}"));
            continue;
        };
        if total > 1 {
            duplicates.push((subject, total));
        }
    }
    duplicates
}

/// Runs `git log <revspec> --format=%s` in `root` and returns non-empty trimmed
/// subject lines, or records a failure and returns `None`.
fn git_subjects(root: &Path, revspec: &str, failures: &mut Vec<String>) -> Option<Vec<String>> {
    let output = match Command::new("git")
        .args(["log", revspec, "--format=%s"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(proc) => proc,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            failures.push("git not found; cannot check commit subjects".to_owned());
            return None;
        }
        Err(err) => {
            failures.push(format!(
                "git invocation failed: {err}; cannot check commit subjects"
            ));
            return None;
        }
    };
    if !output.status.success() {
        failures.push(format!(
            "git log {revspec} exited with {}; cannot check commit subjects (stderr: {})",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(
        stdout
            .lines()
            .map(str::trim)
            .filter(|subject| !subject.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// Counts occurrences of each commit subject with checked arithmetic.
fn count_subjects(
    subjects: &[String],
    failures: &mut Vec<String>,
) -> Option<BTreeMap<String, usize>> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for subject in subjects {
        let current = match counts.get(subject) {
            Some(count) => *count,
            None => 0,
        };
        let Some(next) = current.checked_add(1) else {
            failures.push(format!("commit subject count overflowed for {subject:?}"));
            return None;
        };
        counts.insert(subject.clone(), next);
    }
    Some(counts)
}

/// Resolves which commits to check, or `None` (with a recorded failure) when an
/// explicit `SLUGAUDIT_CHECK_COMMITS_BASE` is invalid or `git` cannot run.
fn commit_scope(
    base: Result<String, VarError>,
    root: &Path,
    failures: &mut Vec<String>,
) -> Option<CommitScope> {
    match base {
        Ok(base) if base.trim().is_empty() => {
            failures.push(
                "SLUGAUDIT_CHECK_COMMITS_BASE is set but empty; cannot choose the commit range"
                    .to_owned(),
            );
            return None;
        }
        Ok(base) => return Some(CommitScope::SinceBase(base.trim().to_owned())),
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
        .args([
            "merge-base",
            "--is-ancestor",
            HISTORICAL_BASELINE_COMMIT,
            "HEAD",
        ])
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Some(CommitScope::SinceBase(
            HISTORICAL_BASELINE_COMMIT.to_owned(),
        )),
        // Either the historical cut commit is absent (e.g. a synthetic test
        // repository) or `root` is not a git checkout (which `git log HEAD`
        // then fails loudly on with stderr).
        Ok(_) => Some(CommitScope::AllReachable),
        Err(err) => {
            failures.push(format!(
                "git merge-base could not run: {err}; cannot choose the commit range"
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

    fn run_git(repo: &Path, args: &[&str]) -> TestResult {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .check("spawn git in test fixture")?;
        ensure(
            output.status.success(),
            format!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }

    fn git_head(repo: &Path) -> TestResult<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .check("git rev-parse HEAD")?;
        ensure(
            output.status.success(),
            format!(
                "git rev-parse HEAD failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    #[test]
    fn a_base_that_is_not_utf8_fails_instead_of_widening_the_range() -> TestResult {
        let mut failures = Vec::new();

        let scope = commit_scope(
            Err(VarError::NotUnicode(OsString::from("x"))),
            Path::new("."),
            &mut failures,
        );

        ensure_eq(&scope, &None, "invalid UTF-8 base must return no scope")?;
        ensure(
            failures.iter().any(|f| f.contains("not valid UTF-8")),
            format!("expected UTF-8 error in {failures:?}"),
        )?;
        Ok(())
    }

    #[test]
    fn a_set_base_selects_its_own_range() -> TestResult {
        let mut failures = Vec::new();

        let scope = commit_scope(Ok("abc".to_owned()), Path::new("."), &mut failures);

        ensure_eq(
            &scope,
            &Some(CommitScope::SinceBase("abc".to_owned())),
            "explicit base selects SinceBase(\"abc\")",
        )?;
        ensure(failures.is_empty(), "valid base produces no failures")?;
        Ok(())
    }

    #[test]
    fn an_empty_or_blank_base_fails_instead_of_checking_no_commits() -> TestResult {
        for base in ["", "  ", "\t\n"] {
            let mut failures = Vec::new();

            let scope = commit_scope(Ok(base.to_owned()), Path::new("."), &mut failures);

            ensure_eq(
                &scope,
                &None,
                format!("blank base {base:?} returns no scope"),
            )?;
            ensure_eq(
                &failures,
                &["SLUGAUDIT_CHECK_COMMITS_BASE is set but empty; cannot choose the commit range"],
                format!("blank base {base:?} records the empty-base error"),
            )?;
        }
        Ok(())
    }

    #[test]
    fn a_failing_git_log_reports_its_stderr() -> TestResult {
        let dir = tempfile::tempdir().check("create temp dir")?;
        let mut failures = Vec::new();

        let subjects = duplicate_commit_subjects(dir.path(), &mut failures);

        ensure(subjects.is_empty(), "failed git log returns no subjects")?;
        let message = failures.first().check("git log outside a repo must fail")?;
        ensure(message.contains("stderr: "), message)?;
        ensure(
            !message.ends_with("stderr: )"),
            format!("stderr was dropped: {message}"),
        )?;
        Ok(())
    }

    #[test]
    fn direct_commits_on_main_with_origin_main_at_head_still_catch_duplicates() -> TestResult {
        let repo = tempfile::tempdir().check("create temp git repo")?;
        run_git(repo.path(), &["init", "-b", "main"])?;
        run_git(
            repo.path(),
            &["commit", "--allow-empty", "-m", "feat: add index"],
        )?;
        run_git(
            repo.path(),
            &["commit", "--allow-empty", "-m", "feat: add index"],
        )?;
        // Simulate a direct push to `main` where `refs/remotes/origin/main`
        // points at `HEAD` (`origin/main..HEAD` is empty).
        let head = git_head(repo.path())?;
        run_git(
            repo.path(),
            &["update-ref", "refs/remotes/origin/main", &head],
        )?;

        let mut failures = Vec::new();
        let scope = commit_scope(Err(VarError::NotPresent), repo.path(), &mut failures)
            .check("commit scope resolves")?;
        ensure_eq(
            &scope,
            &CommitScope::AllReachable,
            "synthetic repo without historical cut scans all reachable commits",
        )?;
        let subjects = git_subjects(repo.path(), "HEAD", &mut failures).check("git log HEAD")?;
        let counts = count_subjects(&subjects, &mut failures).check("subject counts")?;
        let duplicates: Vec<(String, usize)> =
            counts.into_iter().filter(|(_, count)| *count > 1).collect();

        ensure(
            failures.is_empty(),
            format!("unexpected failures: {failures:?}"),
        )?;
        ensure_eq(
            &duplicates,
            &vec![("feat: add index".to_owned(), 2)],
            "duplicate subject on main is detected even when origin/main equals HEAD",
        )?;
        Ok(())
    }

    #[test]
    fn commits_after_base_cannot_reuse_a_pre_base_subject() -> TestResult {
        let repo = tempfile::tempdir().check("create temp git repo")?;
        run_git(repo.path(), &["init", "-b", "main"])?;
        run_git(
            repo.path(),
            &["commit", "--allow-empty", "-m", "old duplicate"],
        )?;
        run_git(
            repo.path(),
            &["commit", "--allow-empty", "-m", "old duplicate"],
        )?;
        run_git(
            repo.path(),
            &["commit", "--allow-empty", "-m", "feat: unique baseline"],
        )?;
        let base = git_head(repo.path())?;

        // Before any new duplicate after `base`, the pre-base pair is ignored.
        let mut failures = Vec::new();
        let clean = duplicates_since_base(repo.path(), &base, &mut failures);
        ensure(
            failures.is_empty(),
            format!("unexpected failures: {failures:?}"),
        )?;
        ensure(
            clean.is_empty(),
            format!("untouched pre-base history must pass, got {clean:?}"),
        )?;

        // Adding a commit after `base` that reuses a pre-base subject fails.
        run_git(
            repo.path(),
            &["commit", "--allow-empty", "-m", "feat: unique baseline"],
        )?;
        let caught = duplicates_since_base(repo.path(), &base, &mut failures);
        ensure(
            failures.is_empty(),
            format!("unexpected failures: {failures:?}"),
        )?;
        ensure_eq(
            &caught,
            &vec![("feat: unique baseline".to_owned(), 2)],
            "reusing a pre-base commit subject after the baseline must fail",
        )?;
        Ok(())
    }

    #[test]
    fn this_repository_has_no_post_baseline_duplicate_commit_subjects() -> TestResult {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut failures = Vec::new();
        let duplicates = duplicates_since_base(&root, HISTORICAL_BASELINE_COMMIT, &mut failures);

        ensure(
            failures.is_empty(),
            format!("git log on this repository must succeed: {failures:?}"),
        )?;
        ensure(
            duplicates.is_empty(),
            format!("duplicate commit subjects since baseline: {duplicates:?}"),
        )?;
        Ok(())
    }
}
