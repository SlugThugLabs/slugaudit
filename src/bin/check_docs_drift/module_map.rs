//! Check 1: the `ARCHITECTURE.md` module-map tree must reference only files
//! and directories that actually exist. Catches renames that were never
//! reflected in the map (e.g. the `publish_edges.rs` → `revision_edges.rs`
//! rename that went stale).

use std::path::{Path, PathBuf};

use super::read;

pub(super) fn check_module_map(root: &Path, failures: &mut Vec<String>) {
    let architecture = read(root, "ARCHITECTURE.md");
    let lines: Vec<&str> = architecture.lines().collect();

    // Find the module map: a fenced block whose first content line is `src/`.
    let mut map_start = None;
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim() == "```" && lines[i + 1].trim() == "src/" {
            map_start = Some(i + 1);
            break;
        }
        i += 1;
    }
    let Some(map_start) = map_start else {
        failures.push("ARCHITECTURE.md module-map block (```src/ ... ```) not found".to_owned());
        return;
    };
    let mut map_end = map_start + 1;
    while map_end < lines.len() && lines[map_end].trim() != "```" {
        map_end += 1;
    }

    // Stack of (directory name, depth). The tree indents 4 columns per
    // level, so depth = connector column / 4 + 1; the `src/` root line is
    // depth 0. The connector column is counted in characters (the `│` bar
    // is multi-byte UTF-8, so a byte offset would misalign).
    let mut stack: Vec<(String, usize)> = vec![("src".to_owned(), 0)];
    for (offset, line) in lines[map_start + 1..map_end].iter().enumerate() {
        let line_no = map_start + 1 + offset + 1;
        let connector = line.find("├──").or_else(|| line.find("└──"));
        let Some(connector) = connector else {
            continue; // comment-continuation or decoration line
        };
        let column = line[..connector].chars().count();
        let depth = column / 4 + 1;
        // Skip the `├──` / `└──` marker (3 chars, but multi-byte in UTF-8,
        // so trim by characters rather than by byte offset).
        let rest = line[connector..]
            .trim_start_matches(['├', '└'])
            .trim_start_matches('─')
            .trim_start();
        let token = rest.split_whitespace().next().unwrap_or("");
        if token.is_empty() {
            continue;
        }
        let is_dir = token.ends_with('/');
        let name = token.trim_end_matches('/');

        while stack.last().is_some_and(|(_, d)| *d >= depth) {
            stack.pop();
        }
        let mut rel = PathBuf::from("src");
        for (dir, _) in stack.iter().skip(1) {
            rel.push(dir);
        }
        rel.push(name);

        // Existence is checked against `root`, not the process CWD: the
        // gate must validate the project it was pointed at (`--root`), even
        // when run from a different working directory. `rel` stays the
        // `src/...` reference for the message; only the check joins it to
        // `root`.
        let checked = root.join(&rel);
        if is_dir {
            if !checked.is_dir() {
                failures.push(format!(
                    "ARCHITECTURE.md line {line_no}: module map references directory `{}` which \
                     does not exist",
                    rel.display()
                ));
            }
            stack.push((name.to_owned(), depth));
        } else if !checked.is_file() {
            failures.push(format!(
                "ARCHITECTURE.md line {line_no}: module map references file `{}` which does not \
                 exist",
                rel.display()
            ));
        }
    }
}
