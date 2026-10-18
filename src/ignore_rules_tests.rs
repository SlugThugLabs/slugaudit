//! Tests for the shared ignore rules. The critical cases: nested
//! `.gitignore` scoping (a single-builder matcher leaks patterns outside
//! their directory), negations, the "can't re-include under an excluded
//! parent" rule, `.ignore` overriding `.gitignore`, and parity with
//! discovery's walker — the exact inconsistency the watcher previously
//! caused.

use super::*;
use crate::sync::discover;
use crate::watch::WatchScope;
use std::fs;

fn project() -> tempfile::TempDir {
    tempfile::tempdir().expect("project dir")
}

/// Builds rules the way production does: scope walk first (collects the
/// ignore files), then `IgnoreRules::build`.
fn rules_for(root: &Path) -> IgnoreRules {
    let scope = WatchScope::compute(root);
    IgnoreRules::build(root, &scope.ignore_files)
}

#[test]
fn a_gitignored_directory_is_ignored() {
    let project = project();
    fs::write(project.path().join(".gitignore"), "target/\n").expect("gitignore");
    fs::create_dir_all(project.path().join("target")).expect("target dir");
    fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").expect("target file");
    fs::create_dir_all(project.path().join("src")).expect("src dir");
    fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").expect("lib file");

    let rules = rules_for(project.path());
    assert!(rules.should_ignore("target/gen.rs"));
    assert!(!rules.should_ignore("src/lib.rs"));
    assert!(
        !rules.should_ignore(".gitignore"),
        "a gitignore file is indexable"
    );
}

#[test]
fn a_nested_gitignore_is_scoped_to_its_directory() {
    let project = project();
    fs::create_dir_all(project.path().join("sub")).expect("sub dir");
    fs::create_dir_all(project.path().join("other")).expect("other dir");
    fs::write(project.path().join("sub/.gitignore"), "*.log\n").expect("nested gitignore");
    fs::write(project.path().join("sub/debug.log"), "x\n").expect("sub log");
    fs::write(project.path().join("debug.log"), "x\n").expect("root log");
    fs::write(project.path().join("other/debug.log"), "x\n").expect("other log");

    let rules = rules_for(project.path());
    assert!(rules.should_ignore("sub/debug.log"));
    // A single-builder matcher would leak `*.log` outside `sub/` — the
    // per-directory scoping must keep these indexable.
    assert!(!rules.should_ignore("debug.log"));
    assert!(!rules.should_ignore("other/debug.log"));
}

#[test]
fn negation_re_includes_a_file() {
    let project = project();
    fs::write(project.path().join(".gitignore"), "*.log\n!keep.log\n").expect("gitignore");
    fs::write(project.path().join("keep.log"), "x\n").expect("keep file");
    fs::write(project.path().join("debug.log"), "x\n").expect("debug file");

    let rules = rules_for(project.path());
    assert!(
        !rules.should_ignore("keep.log"),
        "the negation must re-include"
    );
    assert!(rules.should_ignore("debug.log"));
}

#[test]
fn a_file_under_an_ignored_directory_stays_ignored_even_if_whitelisted() {
    // Git cannot re-include a file when a parent directory is excluded.
    let project = project();
    fs::write(
        project.path().join(".gitignore"),
        "build/\n!build/keep.rs\n",
    )
    .expect("gitignore");
    fs::create_dir_all(project.path().join("build")).expect("build dir");
    fs::write(project.path().join("build/keep.rs"), "fn keep() {}\n").expect("keep file");

    let rules = rules_for(project.path());
    assert!(
        rules.should_ignore("build/keep.rs"),
        "an excluded parent directory prunes the whole subtree"
    );
}

#[test]
fn a_dot_ignore_overrides_dot_gitignore() {
    let project = project();
    fs::write(project.path().join(".gitignore"), "*.log\n").expect("gitignore");
    fs::write(project.path().join(".ignore"), "!keep.log\n").expect("dot ignore");
    fs::write(project.path().join("keep.log"), "x\n").expect("keep file");

    let rules = rules_for(project.path());
    assert!(
        !rules.should_ignore("keep.log"),
        ".ignore must override .gitignore"
    );
    assert!(rules.should_ignore("debug.log"));
}

#[test]
fn hardcoded_exclusions_apply_everywhere() {
    let project = project();
    fs::create_dir_all(project.path().join(".planning/slugaudit")).expect("data dir");
    fs::create_dir_all(project.path().join("src")).expect("src dir");
    fs::write(project.path().join(".planning/slugaudit/project.db"), "db").expect("db");
    fs::write(project.path().join("src/lib.rs~"), "backup").expect("swap file");
    fs::write(project.path().join("notes.tmp"), "tmp").expect("tmp file");

    let rules = rules_for(project.path());
    assert!(rules.should_ignore(".planning/slugaudit/project.db"));
    assert!(rules.should_ignore("src/lib.rs~"));
    assert!(rules.should_ignore("notes.tmp"));
    assert!(!rules.should_ignore("src/lib.rs"));
}

/// The crate documents that any `.ignore` file overrides all `.gitignore`
/// files regardless of directory depth. The matcher must agree with the
/// walker on this cross-type case — a nested `.gitignore` must not beat a
/// root `.ignore` whitelist.
#[test]
fn a_dot_ignore_overrides_a_nested_gitignore() {
    let project = project();
    fs::create_dir_all(project.path().join("sub")).expect("sub dir");
    fs::write(project.path().join(".ignore"), "!keep.log\n").expect("root dot ignore");
    fs::write(project.path().join("sub/.gitignore"), "*.log\n").expect("nested gitignore");
    fs::write(project.path().join("sub/keep.log"), "x\n").expect("keep file");
    fs::write(project.path().join("sub/debug.log"), "x\n").expect("debug file");

    let rules = rules_for(project.path());
    assert!(
        !rules.should_ignore("sub/keep.log"),
        ".ignore overrides .gitignore even at different depths"
    );
    assert!(rules.should_ignore("sub/debug.log"));
}

/// Pins the *actual* walker behavior for the cross-type precedence case
/// against the rules matcher: whatever discovery decides, the rules must
/// agree — that is the whole point of the shared matcher.
#[test]
fn cross_type_precedence_matches_the_walker() {
    let project = project();
    fs::create_dir_all(project.path().join("sub")).expect("sub dir");
    fs::write(project.path().join(".ignore"), "!keep.log\n").expect("root dot ignore");
    fs::write(project.path().join("sub/.gitignore"), "*.log\n").expect("nested gitignore");
    fs::write(project.path().join("sub/keep.log"), "x\n").expect("keep file");
    fs::write(project.path().join("sub/debug.log"), "x\n").expect("debug file");

    let (files, _skipped) = discover(project.path()).expect("discover");
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
}

/// The heart of the fix: incremental reconcile must skip exactly what a
/// fresh `discover()` would skip. Every discovered file must be allowed
/// by the rules, and every rule-ignored file must be absent from
/// discovery — otherwise the watcher path and the publish path disagree
/// about the same tree.
#[test]
fn discovery_parity_rules_and_walker_agree_on_every_file() {
    let project = project();
    fs::write(project.path().join(".gitignore"), "target/\n*.log\n").expect("gitignore");
    fs::create_dir_all(project.path().join("target")).expect("target dir");
    fs::write(project.path().join("target/gen.rs"), "fn gen() {}\n").expect("target file");
    fs::create_dir_all(project.path().join("sub")).expect("sub dir");
    fs::write(project.path().join("sub/.gitignore"), "secret.rs\n").expect("nested gitignore");
    fs::write(project.path().join("sub/secret.rs"), "fn s() {}\n").expect("secret file");
    fs::write(project.path().join("sub/open.rs"), "fn o() {}\n").expect("open file");
    fs::write(project.path().join("debug.log"), "x\n").expect("log file");
    fs::create_dir_all(project.path().join("src")).expect("src dir");
    fs::write(project.path().join("src/lib.rs"), "fn lib() {}\n").expect("lib file");

    let (files, _skipped) = discover(project.path()).expect("discover");
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
fn an_unstattable_git_exclude_is_reported_not_skipped_silently() {
    let project = project();
    fs::write(project.path().join(".gitignore"), "*.tmp\n").expect("gitignore");
    // A linked worktree or submodule has `.git` as a file, so
    // `.git/info/exclude` fails to stat with "not a directory".
    fs::write(project.path().join(".git"), "gitdir: elsewhere\n").expect("git file");

    let (_rules, logs) = crate::store::test_capture::capture_warns(|| rules_for(project.path()));

    assert!(
        logs.contains("could not stat .git/info/exclude"),
        "stat failure must be logged:\n{logs}"
    );
}
