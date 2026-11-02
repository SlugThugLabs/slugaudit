//! Tests for the shared ignore rules. The critical cases: nested
//! `.gitignore` scoping (a single-builder matcher leaks patterns outside
//! their directory), negations, the "can't re-include under an excluded
//! parent" rule, `.ignore` overriding `.gitignore`, and parity with
//! discovery's walker.

use super::*;
use crate::sync::discover;
use crate::util::TEST_ENV_LOCK;
use slugaudit_test_check::{Check, TestResult};
use std::fs;

fn project() -> TestResult<tempfile::TempDir> {
    tempfile::tempdir().check("project dir")
}

/// The structured ignore-rule build records for one source in a captured
/// log; there is one per failure.
fn build_failure_records<'a>(logs: &'a str, source: &str) -> Vec<&'a str> {
    let source_field = format!("source=\"{source}\"");
    logs.lines()
        .filter(|line| {
            line.contains("operation=\"ignore_rules_build\"") && line.contains(&source_field)
        })
        .collect()
}

/// A capture that went blind would make every absence assertion below pass
/// vacuously; each test emits this control line inside its closure.
fn assert_capture_live(logs: &str) {
    assert!(
        logs.contains("capture liveness control for ignore_rules_build"),
        "the capture must see this thread's events, captured: {logs}"
    );
}

/// Builds rules the way production does: walk the project to collect its
/// ignore files, then `IgnoreRules::build`.
fn rules_for(root: &Path) -> IgnoreRules {
    let mut ignore_files = Vec::new();
    for entry in indexable_walker(root) {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if name == ".gitignore" || name == ".ignore" {
            ignore_files.push(entry.path().to_path_buf());
        }
    }
    IgnoreRules::build(root, &ignore_files)
}

#[test]
fn a_gitignored_directory_is_ignored() -> TestResult {
    let project = project()?;
    fs::write(project.path().join(".gitignore"), "target/\n").check("gitignore")?;
    fs::create_dir_all(project.path().join("target")).check("target dir")?;
    fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").check("target file")?;
    fs::create_dir_all(project.path().join("src")).check("src dir")?;
    fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").check("lib file")?;

    let rules = rules_for(project.path());
    assert!(rules.should_ignore("target/gen.rs"));
    assert!(!rules.should_ignore("src/lib.rs"));
    assert!(
        !rules.should_ignore(".gitignore"),
        "a gitignore file is indexable"
    );
    Ok(())
}

#[test]
fn a_nested_gitignore_is_scoped_to_its_directory() -> TestResult {
    let project = project()?;
    fs::create_dir_all(project.path().join("sub")).check("sub dir")?;
    fs::create_dir_all(project.path().join("other")).check("other dir")?;
    fs::write(project.path().join("sub/.gitignore"), "*.log\n").check("nested gitignore")?;
    fs::write(project.path().join("sub/debug.log"), "x\n").check("sub log")?;
    fs::write(project.path().join("debug.log"), "x\n").check("root log")?;
    fs::write(project.path().join("other/debug.log"), "x\n").check("other log")?;

    let rules = rules_for(project.path());
    assert!(rules.should_ignore("sub/debug.log"));
    // A single-builder matcher would leak `*.log` outside `sub/` — the
    // per-directory scoping must keep these indexable.
    assert!(!rules.should_ignore("debug.log"));
    assert!(!rules.should_ignore("other/debug.log"));
    Ok(())
}

#[test]
fn negation_re_includes_a_file() -> TestResult {
    let project = project()?;
    fs::write(project.path().join(".gitignore"), "*.log\n!keep.log\n").check("gitignore")?;
    fs::write(project.path().join("keep.log"), "x\n").check("keep file")?;
    fs::write(project.path().join("debug.log"), "x\n").check("debug file")?;

    let rules = rules_for(project.path());
    assert!(
        !rules.should_ignore("keep.log"),
        "the negation must re-include"
    );
    assert!(rules.should_ignore("debug.log"));
    Ok(())
}

#[test]
fn a_file_under_an_ignored_directory_stays_ignored_even_if_whitelisted() -> TestResult {
    // Git cannot re-include a file when a parent directory is excluded.
    let project = project()?;
    fs::write(
        project.path().join(".gitignore"),
        "build/\n!build/keep.rs\n",
    )
    .check("gitignore")?;
    fs::create_dir_all(project.path().join("build")).check("build dir")?;
    fs::write(project.path().join("build/keep.rs"), "fn keep() {}\n").check("keep file")?;

    let rules = rules_for(project.path());
    assert!(
        rules.should_ignore("build/keep.rs"),
        "an excluded parent directory prunes the whole subtree"
    );
    Ok(())
}

#[test]
fn a_dot_ignore_overrides_dot_gitignore() -> TestResult {
    let project = project()?;
    fs::write(project.path().join(".gitignore"), "*.log\n").check("gitignore")?;
    fs::write(project.path().join(".ignore"), "!keep.log\n").check("dot ignore")?;
    fs::write(project.path().join("keep.log"), "x\n").check("keep file")?;

    let rules = rules_for(project.path());
    assert!(
        !rules.should_ignore("keep.log"),
        ".ignore must override .gitignore"
    );
    assert!(rules.should_ignore("debug.log"));
    Ok(())
}

#[test]
fn hardcoded_exclusions_apply_everywhere() -> TestResult {
    let project = project()?;
    fs::create_dir_all(project.path().join(".planning/slugaudit")).check("data dir")?;
    fs::create_dir_all(project.path().join("src")).check("src dir")?;
    fs::write(project.path().join(".planning/slugaudit/project.db"), "db").check("db")?;
    fs::write(project.path().join("src/lib.rs~"), "backup").check("swap file")?;
    fs::write(project.path().join("notes.tmp"), "tmp").check("tmp file")?;

    let rules = rules_for(project.path());
    assert!(rules.should_ignore(".planning/slugaudit/project.db"));
    assert!(rules.should_ignore("src/lib.rs~"));
    assert!(rules.should_ignore("notes.tmp"));
    assert!(!rules.should_ignore("src/lib.rs"));
    Ok(())
}

/// The crate documents that any `.ignore` file overrides all `.gitignore`
/// files regardless of directory depth. The matcher must agree with the
/// walker on this cross-type case — a nested `.gitignore` must not beat a
/// root `.ignore` whitelist.
#[test]
fn a_dot_ignore_overrides_a_nested_gitignore() -> TestResult {
    let project = project()?;
    fs::create_dir_all(project.path().join("sub")).check("sub dir")?;
    fs::write(project.path().join(".ignore"), "!keep.log\n").check("root dot ignore")?;
    fs::write(project.path().join("sub/.gitignore"), "*.log\n").check("nested gitignore")?;
    fs::write(project.path().join("sub/keep.log"), "x\n").check("keep file")?;
    fs::write(project.path().join("sub/debug.log"), "x\n").check("debug file")?;

    let rules = rules_for(project.path());
    assert!(
        !rules.should_ignore("sub/keep.log"),
        ".ignore overrides .gitignore even at different depths"
    );
    assert!(rules.should_ignore("sub/debug.log"));
    Ok(())
}

/// Pins the *actual* walker behavior for the cross-type precedence case
/// against the rules matcher: whatever discovery decides, the rules must
/// agree — that is the whole point of the shared matcher.
#[test]
fn cross_type_precedence_matches_the_walker() -> TestResult {
    let project = project()?;
    fs::create_dir_all(project.path().join("sub")).check("sub dir")?;
    fs::write(project.path().join(".ignore"), "!keep.log\n").check("root dot ignore")?;
    fs::write(project.path().join("sub/.gitignore"), "*.log\n").check("nested gitignore")?;
    fs::write(project.path().join("sub/keep.log"), "x\n").check("keep file")?;
    fs::write(project.path().join("sub/debug.log"), "x\n").check("debug file")?;

    let (files, _skipped) = discover(project.path()).check("discover")?;
    let indexed: std::collections::HashSet<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    let rules = rules_for(project.path());

    for file in &files {
        assert!(
            !rules.should_ignore(&file.relative_path),
            "discovery indexed {:?} but the rules ignore it",
            file.relative_path,
        );
    }
    for path in ["sub/keep.log", "sub/debug.log"] {
        assert_eq!(
            indexed.contains(path),
            !rules.should_ignore(path),
            "rules and discovery must agree on {path}",
        );
    }
    Ok(())
}

/// The walker must not apply parent-directory ignore files above the
/// project root; project-scoped rules are the contract.
#[test]
fn a_parent_gitignore_does_not_diverge_from_the_incremental_rules() -> TestResult {
    let repo = project()?;
    fs::write(repo.path().join(".gitignore"), "dist/\n").check("parent gitignore")?;
    let root = repo.path().join("nested-project");
    fs::create_dir_all(root.join("dist")).check("dist dir")?;
    fs::write(root.join("dist/out.rs"), "fn out() {}\n").check("dist file")?;
    fs::create_dir_all(root.join("src")).check("src dir")?;
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").check("lib file")?;

    let (files, _skipped) = discover(&root).check("discover")?;
    let indexed: std::collections::HashSet<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    let rules = rules_for(&root);

    assert!(
        indexed.contains("dist/out.rs"),
        "the walker must not apply a parent ignore file: {indexed:?}"
    );
    assert!(
        !rules.should_ignore("dist/out.rs"),
        "the incremental rules must agree with the walker"
    );
    Ok(())
}

/// Excluded directories must be pruned at descent time. Filtering their
/// yielded files keeps them out of the index, but walking a large `.git` or
/// data directory still costs time on every sync.
#[test]
fn the_walker_does_not_descend_into_excluded_directories() -> TestResult {
    let project = project()?;
    let root = project.path();
    fs::create_dir_all(root.join(".git/objects")).check("git objects")?;
    for index in 0..32 {
        fs::write(root.join(format!(".git/objects/{index:02}")), "x").check("git object")?;
    }
    fs::create_dir_all(root.join(".planning/slugaudit")).check("data dir")?;
    for index in 0..32 {
        fs::write(root.join(format!(".planning/slugaudit/{index:02}.db")), "x").check("data")?;
    }
    fs::create_dir_all(root.join("src")).check("src dir")?;
    fs::write(root.join("src/lib.rs"), "fn lib() {}\n").check("lib")?;

    let yielded: Vec<std::path::PathBuf> = indexable_walker(root)
        .map(|entry| entry.map(|entry| entry.path().to_path_buf()))
        .collect::<Result<_, _>>()
        .check("walk the project")?;

    assert!(
        yielded
            .iter()
            .all(|path| !path.starts_with(root.join(".git"))
                && !path.starts_with(root.join(".planning/slugaudit"))),
        "excluded subtrees must not be yielded: {yielded:?}"
    );
    assert!(yielded.iter().any(|path| path.ends_with("src/lib.rs")));
    Ok(())
}

/// The rules matcher and the walker must agree on every file: every
/// discovered file is allowed by the rules, and every rule-ignored file is
/// absent from discovery.
#[test]
fn discovery_parity_rules_and_walker_agree_on_every_file() -> TestResult {
    let project = project()?;
    fs::write(project.path().join(".gitignore"), "target/\n*.log\n").check("gitignore")?;
    fs::create_dir_all(project.path().join("target")).check("target dir")?;
    fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").check("target file")?;
    fs::create_dir_all(project.path().join("sub")).check("sub dir")?;
    fs::write(project.path().join("sub/.gitignore"), "secret.rs\n").check("nested gitignore")?;
    fs::write(project.path().join("sub/secret.rs"), "fn s() {}\n").check("secret file")?;
    fs::write(project.path().join("sub/open.rs"), "fn o() {}\n").check("open file")?;
    fs::write(project.path().join("debug.log"), "x\n").check("log file")?;
    fs::create_dir_all(project.path().join("src")).check("src dir")?;
    fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").check("lib file")?;

    let (files, _skipped) = discover(project.path()).check("discover")?;
    let rules = rules_for(project.path());

    for file in &files {
        assert!(
            !rules.should_ignore(&file.relative_path),
            "discovery indexed {:?} but the rules ignore it",
            file.relative_path,
        );
    }

    let indexed: std::collections::HashSet<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    for ignored in ["target/gen.rs", "debug.log", "sub/secret.rs"] {
        assert!(
            !indexed.contains(ignored),
            "{ignored} must not be discovered"
        );
    }
    assert!(indexed.contains("sub/open.rs"));
    assert!(indexed.contains("src/lib.rs"));
    assert!(indexed.contains(".gitignore"));
    assert!(indexed.contains("sub/.gitignore"));
    Ok(())
}

#[test]
fn the_runtime_data_directory_and_its_descendants_are_excluded() {
    for excluded in [
        ".planning/slugaudit",
        ".planning/slugaudit/project.db",
        ".planning/slugaudit/nested/deep.json",
        ".planning\\slugaudit\\project.db",
    ] {
        assert!(is_excluded_path(excluded), "{excluded} must be excluded");
        assert!(is_excluded_dir(excluded), "{excluded} must be excluded");
    }
}

#[test]
fn a_planning_sibling_sharing_the_runtime_dir_prefix_stays_indexed() {
    for included in [
        ".planning/slugaudit-notes/x.md",
        ".planning/slugaudit-notes",
        ".planning/slugauditx",
        ".planning/slugauditfoo/y.md",
        ".planning/other/z.md",
        ".planning/PLAN.md",
        ".planning",
        "slugaudit/x.md",
    ] {
        assert!(!is_excluded_path(included), "{included} must be indexed");
    }
}

#[test]
fn an_ignore_file_without_a_parent_is_skipped_with_a_warning() {
    let (grouped, logs) = crate::store::test_capture::capture_warns(|| {
        build::group_files_by_dir(&[PathBuf::from("/")])
    });

    assert!(grouped.is_empty());
    assert!(logs.contains("has no parent directory"), "{logs}");
}

#[test]
fn an_unstattable_git_exclude_is_reported_not_skipped_silently() -> TestResult {
    let project = project()?;
    fs::write(project.path().join(".gitignore"), "*.tmp\n").check("gitignore")?;
    // A linked worktree or submodule has `.git` as a file, so
    // `.git/info/exclude` fails to stat with "not a directory".
    fs::write(project.path().join(".git"), "gitdir: elsewhere\n").check("git file")?;

    let (_rules, logs) = crate::store::test_capture::capture_warns(|| rules_for(project.path()));

    assert!(
        logs.contains("could not stat .git/info/exclude"),
        "stat failure must be logged:\n{logs}"
    );
    Ok(())
}

/// `ignore::Error`'s `Display` for a `WithPath`/`Glob` error is
/// `"<absolute ignore-file path>: line N: error parsing glob '<glob>': ..."`,
/// so a malformed project `.gitignore` must emit structured fields rather
/// than the error text.
#[test]
fn a_malformed_project_gitignore_logs_the_path_but_no_glob_text() -> TestResult {
    let project = project()?;
    let gitignore = project.path().join(".gitignore");
    fs::write(&gitignore, "[z-a]\n").check("malformed gitignore")?;
    let expected_path = gitignore.to_string_lossy().to_string();

    let (_rules, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for ignore_rules_build");
        IgnoreRules::build(project.path(), std::slice::from_ref(&gitignore))
    });

    assert_capture_live(&logs);
    let records = build_failure_records(&logs, "project_gitignore");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("source=\"project_gitignore\""),
        "the source must be named:\n{logs}"
    );
    assert!(
        records[0].contains("error_kind=\"glob\""),
        "the error category must be named:\n{logs}"
    );
    assert!(
        records[0].contains(&expected_path),
        "the real path must be present:\n{logs}"
    );
    assert!(!logs.contains("[z-a]"), "the glob text leaked:\n{logs}");
    Ok(())
}

/// The global gitignore normally lives under `$HOME`/XDG, so the same
/// error `Display` would put the operator's home directory in a server log.
#[test]
fn a_malformed_global_gitignore_logs_the_path_but_no_glob_text() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let home = project()?;
    let xdg = tempfile::tempdir().check("xdg config dir")?;
    let global = xdg.path().join("git/ignore");
    fs::create_dir_all(global.parent().check("git config dir")?).check("git config dir")?;
    fs::write(&global, "[z-a]\n").check("malformed global ignore")?;
    let missing_config = home.path().join("absent-gitconfig");
    let expected_path = global.to_string_lossy().to_string();

    let (_rules, logs) = temp_env::with_vars(
        [
            ("HOME", Some(home.path().as_os_str())),
            ("XDG_CONFIG_HOME", Some(xdg.path().as_os_str())),
            // Keep machine-level git config, which could name another
            // `core.excludesFile`, out of the resolution path.
            ("GIT_CONFIG_GLOBAL", Some(missing_config.as_os_str())),
            ("GIT_CONFIG_SYSTEM", Some(missing_config.as_os_str())),
        ],
        || {
            crate::store::test_capture::capture_warns(|| {
                tracing::warn!("capture liveness control for ignore_rules_build");
                IgnoreRules::build(home.path(), &[])
            })
        },
    );

    assert_capture_live(&logs);
    let records = build_failure_records(&logs, "global_gitignore");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("source=\"global_gitignore\""),
        "the source must be named:\n{logs}"
    );
    assert!(
        records[0].contains("error_kind=\"glob\""),
        "the error category must be named:\n{logs}"
    );
    assert!(
        records[0].contains(&expected_path),
        "the real path must be present:\n{logs}"
    );
    assert!(!logs.contains("[z-a]"), "the glob text leaked:\n{logs}");
    Ok(())
}

/// `.git/info/exclude` is added from an absolute path (`root.join`), so
/// its malformed glob must not reach the log either.
#[test]
fn a_malformed_git_exclude_logs_the_path_but_no_glob_text() -> TestResult {
    let project = project()?;
    let gitignore = project.path().join(".gitignore");
    fs::write(&gitignore, "*.tmp\n").check("valid gitignore")?;
    let exclude = project.path().join(".git/info/exclude");
    fs::create_dir_all(exclude.parent().check("exclude dir")?).check("git info dir")?;
    fs::write(&exclude, "[z-a]\n").check("malformed exclude")?;
    let expected_path = exclude.to_string_lossy().to_string();

    let (_rules, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for ignore_rules_build");
        IgnoreRules::build(project.path(), std::slice::from_ref(&gitignore))
    });

    assert_capture_live(&logs);
    let records = build_failure_records(&logs, "git_exclude");
    assert_eq!(records.len(), 1, "one failure is one record:\n{logs}");
    assert!(
        records[0].contains("source=\"git_exclude\""),
        "the source must be named:\n{logs}"
    );
    assert!(
        records[0].contains("error_kind=\"glob\""),
        "the error category must be named:\n{logs}"
    );
    assert!(
        records[0].contains(&expected_path),
        "the real path must be present:\n{logs}"
    );
    assert!(!logs.contains("[z-a]"), "the glob text leaked:\n{logs}");
    Ok(())
}

/// `Error::Partial` joins its members with newlines, so each failure
/// must be logged as its own one-line record rather than one multi-line
/// field.
#[test]
fn a_partially_malformed_gitignore_logs_one_record_per_failure() -> TestResult {
    let project = project()?;
    let gitignore = project.path().join(".gitignore");
    fs::write(&gitignore, "[z-a]\n[9-0]\n").check("malformed gitignore")?;
    let expected_path = gitignore.to_string_lossy().to_string();

    let (_rules, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("capture liveness control for ignore_rules_build");
        IgnoreRules::build(project.path(), std::slice::from_ref(&gitignore))
    });

    assert_capture_live(&logs);
    let records = build_failure_records(&logs, "project_gitignore");
    assert_eq!(records.len(), 2, "one record per failure:\n{logs}");
    for record in records {
        assert!(
            record.contains("source=\"project_gitignore\""),
            "the source must be named:\n{logs}"
        );
        assert!(
            record.contains("error_kind=\"glob\""),
            "the error category must be named:\n{logs}"
        );
        assert!(
            record.contains(&expected_path),
            "the real path must be present:\n{logs}"
        );
    }
    assert!(
        !logs.contains("[z-a]") && !logs.contains("[9-0]"),
        "the glob text leaked:\n{logs}"
    );
    Ok(())
}
