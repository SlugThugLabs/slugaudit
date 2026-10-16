//! Check 2: every literal path a `CODEOWNERS` rule references must exist.
//!
//! A `CODEOWNERS` pattern is allowed to be a glob (`*`, `*.rs`, `/src/**`), a
//! negation (`!pattern`), or a bare suffix (`docs/`) that matches at any
//! depth. None of those can be existence-checked as one path, so the gate
//! verifies only the patterns that name an explicit root-relative path — the
//! form this repository's own `CODEOWNERS` uses. A glob is skipped rather
//! than treated as passing: the check is scoped to what it can decide, and
//! its failure message names the line so the rest is a human decision.

use std::path::Path;

pub(super) fn check_codeowners(root: &Path, failures: &mut Vec<String>) {
    let path = root.join(".github/CODEOWNERS");
    // An absent file is check 1's verdict; this check must not double-report
    // it as a path failure, but must report read errors if present.
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            failures.push(format!(".github/CODEOWNERS is unreadable: {err}"));
            return;
        }
    };
    for (offset, raw) in text.lines().enumerate() {
        let line_no = offset + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(pattern) = line.split_whitespace().next() else {
            continue;
        };
        // Only explicit root-relative paths are decidable; skip globs,
        // negations, and suffix patterns.
        if !pattern.starts_with('/') || pattern.contains(['*', '?', '[', '!']) {
            continue;
        }
        let relative = pattern.trim_start_matches('/').trim_end_matches('/');
        if relative.is_empty() {
            continue;
        }
        if !root.join(relative).exists() {
            failures.push(format!(
                "CODEOWNERS line {line_no}: pattern `{pattern}` references `{relative}`, which \
                 does not exist"
            ));
        }
    }
}
