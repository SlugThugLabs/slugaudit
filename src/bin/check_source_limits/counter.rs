//! Token-aware Rust code-line counter.
//!
//! Faithful port of the prior shell script's state machine: knows about
//! `//` and `/*…*/` comments (the latter nested), `"…"` strings (with
//! `\` escapes but not raw newlines — Rust strings don't have unescaped
//! newlines), `'…'` char literals (including `\\X` escapes), and
//! `r#+"…"#+` raw strings.
//!
//! The state machines's token-coverage semantics: the count measures the
//! number of source lines on which at least one non-comment, non-string
//! Rust token appears. Multi-line raw-string literals count once per
//! physical line that contains a token (mirrors the prior Python
//! implementation: finalize-on-newline runs at the top of the loop
//! regardless of state, so each `\n` inside a raw string still resets
//! `has_code`).

use std::path::{Path, PathBuf};

use crate::scanner::{self, State};

pub(crate) fn code_lines(source: &str) -> usize {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut count: usize = 0;
    let mut has_code = false;
    let mut index: usize = 0;
    let mut block_depth: usize = 0;
    let mut raw_hashes: usize = 0;
    let mut state = State::Normal;

    while index < len {
        scanner::scan_step(
            bytes,
            source,
            &mut index,
            &mut state,
            &mut has_code,
            &mut count,
            &mut block_depth,
            &mut raw_hashes,
        );
    }
    if has_code {
        count += 1;
    }
    count
}

/// The gate's line ceilings. Test files get a higher ceiling because each
/// `#[test]` names one behavior contract — repetition that cannot be
/// DRY'd without losing the failure diagnostics that make a red test
/// actionable — while production code is expected to earn its length.
pub(crate) const TEST_FILE_CEILING: usize = 500;
pub(crate) const PRODUCTION_CEILING: usize = 300;
/// A dated human approval may extend the soft ceiling by this bounded
/// amount; even explicit approval cannot rescue a genuinely oversized file.
pub(crate) const HUMAN_APPROVED_CEILING: usize = 350;
/// Production files at or above this length require an approved
/// `slugaudit-line-exception:` comment.
pub(crate) const PRODUCTION_EXCEPTION_FLOOR: usize = 200;

/// How `main` reports a single file: within limits (optionally via an
/// approved exception) or a violation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Pass,
    PassWithException { reason: String },
    FailHard { ceiling: usize },
    FailNeedsException,
}

/// The gate's policy. Test files (`*_tests.rs`, `tests.rs`) auto-pass up
/// to [`TEST_FILE_CEILING`] with no exception requirement; above it they
/// hard-fail. Production files keep the 0–199 auto-pass / 200–300
/// approved-exception / >300 hard-fail rule, and an exception never
/// rescues a file past [`PRODUCTION_CEILING`].
pub(crate) fn verdict(code_lines: usize, exception: Option<String>, test_file: bool) -> Verdict {
    if test_file {
        if code_lines > TEST_FILE_CEILING {
            return Verdict::FailHard {
                ceiling: TEST_FILE_CEILING,
            };
        }
        return Verdict::Pass;
    }
    if code_lines > PRODUCTION_CEILING {
        if let Some(reason) = exception
            && code_lines <= HUMAN_APPROVED_CEILING
            && reason.starts_with("[human-approved ")
        {
            return Verdict::PassWithException { reason };
        }
        return Verdict::FailHard {
            ceiling: PRODUCTION_CEILING,
        };
    }
    if code_lines >= PRODUCTION_EXCEPTION_FLOOR {
        return match exception {
            Some(reason) => Verdict::PassWithException { reason },
            None => Verdict::FailNeedsException,
        };
    }
    Verdict::Pass
}

pub(crate) fn walk_rs_files(src_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_rs(src_root, &mut out);
    out.sort();
    out
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
