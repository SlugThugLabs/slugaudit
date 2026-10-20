//! Check 1: the `ARCHITECTURE.md` module-map tree must reference only files
//! and directories that actually exist. Catches renames that were never
//! reflected in the map (e.g. the `publish_edges.rs` → `revision_edges.rs`
//! rename that went stale).

use std::path::{Path, PathBuf};

use super::read;

pub(super) fn check_module_map(root: &Path, failures: &mut Vec<String>) {
    let module_map_path = root.join(".planning").join("MODULE_MAP.md");
    let doc_name = if module_map_path.is_file() {
        "MODULE_MAP.md"
    } else {
        "ARCHITECTURE.md"
    };
    let Ok(content) = read(root, doc_name) else {
        failures.push(format!("failed to read {doc_name}"));
        return;
    };
    let lines: Vec<&str> = content.lines().collect();

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
        failures.push(format!(
            "{doc_name} module-map block (```src/ ... ```) not found"
        ));
        return;
    };
    // An unterminated fence used to run `map_end` to EOF, so a truncated map
    // was accepted as if its last line were the end of the block. Without a
    // closing fence there is no block boundary to check entries against;
    // that is a failure of the gate, not a reason to skip it.
    let Some(map_end) = (map_start + 1..lines.len()).find(|&line| lines[line].trim() == "```")
    else {
        failures.push(format!(
            "{doc_name} line {map_start}: module-map block opened by the ``` fence above `src/` \
             is never closed"
        ));
        return;
    };

    // Stack of (directory name, depth). The tree indents 4 columns per
    // level, so depth = connector column / 4 + 1; the `src/` root line is
    // depth 0. The connector column is counted in characters (the `│` bar
    // is multi-byte UTF-8, so a byte offset would misalign).
    let mut stack: Vec<(String, usize)> = vec![("src".to_owned(), 0)];
    let mut parsed_paths = 0;
    for (offset, line) in lines[map_start + 1..map_end].iter().enumerate() {
        let line_no = map_start + 1 + offset + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || is_explicit_heading(trimmed) || is_tree_decoration(trimmed) {
            continue;
        }
        let Some((depth, token)) = parse_tree_entry(line) else {
            // Silently skipping an unrecognized line is how a reformatted
            // tree (`|-- lib.rs`) verified zero paths while printing PASS.
            failures.push(format!(
                "{doc_name} line {line_no}: unparsable module-map line `{trimmed}`; expected a \
                 `├──`/`└──` tree entry"
            ));
            continue;
        };
        let is_dir = token.ends_with('/');
        let name = token.trim_end_matches('/');
        if name.is_empty() || name.contains('/') {
            failures.push(format!(
                "{doc_name} line {line_no}: unparsable module-map entry `{token}`; expected a \
                 single file or directory name"
            ));
            continue;
        }
        // A depth jump would build the wrong parent path for this entry
        // (and every child under it), so fail closed instead of checking a
        // path the tree never named.
        if depth > stack.last().map_or(0, |(_, d)| d + 1) {
            failures.push(format!(
                "{doc_name} line {line_no}: module-map entry `{token}` is indented past its \
                 parent"
            ));
            continue;
        }

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
                    "{doc_name} line {line_no}: module map references directory `{}` which \
                     does not exist",
                    rel.display()
                ));
            }
            stack.push((name.to_owned(), depth));
        } else if !checked.is_file() {
            failures.push(format!(
                "{doc_name} line {line_no}: module map references file `{}` which does not \
                 exist",
                rel.display()
            ));
        }
        parsed_paths += 1;
    }
    // A block the parser understood nothing in has verified nothing. Report
    // that even when every line was "recognized" (for example a heading-only
    // block), so the gate cannot pass without having checked a path.
    if parsed_paths == 0 {
        failures.push(format!(
            "{doc_name}: module-map block contains no tree entries; nothing was verified"
        ));
    }
}

/// Recognized non-entry line: an explicit Markdown heading (`# ...` through
/// `###### ...`), which may group a long tree. It references no path.
fn is_explicit_heading(trimmed: &str) -> bool {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ')
}

/// Recognized non-entry line: a tree decoration carrying only vertical
/// connectors (`│` and spaces), used to separate groups in the map. It
/// references no path, so it neither verifies nor fails anything.
fn is_tree_decoration(trimmed: &str) -> bool {
    trimmed.chars().all(|c| c == '│' || c == ' ')
}

/// Parse one tree-entry line into `(depth, token)`, or `None` when the line
/// does not have the documented shape: an indentation prefix of `│` bars and
/// spaces (four characters per level), a `├──`/`└──` connector, and a
/// non-empty entry token. Anything else fails the check instead of being
/// skipped, so a reformatted tree cannot verify zero paths and pass.
fn parse_tree_entry(line: &str) -> Option<(usize, &str)> {
    let connector = line.find("├──").or_else(|| line.find("└──"))?;
    let prefix = &line[..connector];
    if !prefix.chars().all(|c| c == '│' || c == ' ') {
        return None;
    }
    let column = prefix.chars().count();
    if !column.is_multiple_of(4) {
        return None;
    }
    // Skip the `├──` / `└──` marker (multi-byte in UTF-8, so trim by
    // characters rather than by byte offset).
    let rest = line[connector..]
        .trim_start_matches(['├', '└'])
        .trim_start_matches('─')
        .trim_start();
    let token = rest.split_whitespace().next()?;
    Some((column / 4 + 1, token))
}
