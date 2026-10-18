//! Check 2: `ARCHITECTURE.md`'s `*_tests.rs` count claim must match the
//! real number of `*_tests.rs` files under `src/`.

use std::fs;
use std::path::Path;

use super::read;

pub(super) fn check_test_file_count(root: &Path, failures: &mut Vec<String>) {
    let Ok(architecture) = read(root, "ARCHITECTURE.md") else {
        failures.push("failed to read ARCHITECTURE.md".to_owned());
        return;
    };
    // "The 31 `*_tests.rs` files share this pattern; ..."
    let Some(marker) = architecture.find("`*_tests.rs` files share") else {
        // Returning here made the check pass vacuously the moment the prose
        // was reworded: `main` still printed `docs-drift: PASS` while this
        // third of the gate verified nothing. If the claim is gone there is
        // nothing to check against, and that is a failure of the gate, not a
        // reason to skip it.
        failures.push(
            "ARCHITECTURE.md no longer states the `*_tests.rs` files-share count, so the              count this check exists to verify is unverified — restore the sentence or              remove this check"
                .to_owned(),
        );
        return;
    };
    let before = &architecture[..marker];
    let number = before
        .split_whitespace()
        .rev()
        .find_map(|w| w.trim_end_matches('.').parse::<usize>().ok());

    let src = root.join("src");
    // A missing src/ legitimately holds zero test files; an unreadable one does not.
    let actual = if src.exists() {
        match count_test_files(&src) {
            Ok(n) => n,
            Err(err) => {
                failures.push(format!("cannot count `*_tests.rs` files: {err}"));
                return;
            }
        }
    } else {
        0
    };
    match number {
        Some(claimed) if claimed != actual => failures.push(format!(
            "ARCHITECTURE.md claims {claimed} `*_tests.rs` files, but {actual} exist under src/; \
             update the prose"
        )),
        Some(_) => {}
        None => failures
            .push("ARCHITECTURE.md `*_tests.rs` count claim has no number before it".to_owned()),
    }
}

/// Count `*_tests.rs` files under `dir`. An unreadable directory or entry is an
/// error, not a zero: an undercount next to a stale claim would pass the gate.
fn count_test_files(dir: &Path) -> Result<usize, String> {
    let entries =
        fs::read_dir(dir).map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    let mut count = 0;
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("cannot read an entry of {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            count += count_test_files(&path)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_tests.rs"))
        {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_directory_is_an_error_not_a_zero_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("not_a_dir");
        fs::write(&file, "x").expect("fixture");

        let result = count_test_files(&file);

        assert!(result.is_err(), "{result:?}");
    }
}
