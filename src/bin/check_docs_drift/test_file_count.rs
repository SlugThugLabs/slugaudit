//! Check 2: `ARCHITECTURE.md`'s `*_tests.rs` count claim must match the
//! real number of `*_tests.rs` files under `src/`.

use std::fs;
use std::path::Path;

use super::read;

pub(super) fn check_test_file_count(root: &Path, failures: &mut Vec<String>) {
    let architecture = read(root, "ARCHITECTURE.md");
    // "The 31 `*_tests.rs` files share this pattern; ..."
    let Some(marker) = architecture.find("`*_tests.rs` files share") else {
        return; // claim not present — nothing to verify
    };
    let before = &architecture[..marker];
    let number = before
        .split_whitespace()
        .rev()
        .find_map(|w| w.trim_end_matches('.').parse::<usize>().ok());

    let actual = count_test_files(&root.join("src"));
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

fn count_test_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_test_files(&path);
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_tests.rs"))
        {
            count += 1;
        }
    }
    count
}
