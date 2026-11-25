//! Token-aware Rust code-line counter.
//!
//! State machine that knows about
//! `//` and `/*…*/` comments (the latter nested), `"…"` strings (with
//! `\` escapes but not raw newlines — Rust strings don't have unescaped
//! newlines), `'…'` char literals (including `\\X` escapes), and
//! `r#+"…"#+` raw strings.
//!
//! The state machines's token-coverage semantics: the count measures the
//! number of source lines on which at least one non-comment, non-string
//! Rust token appears. Multi-line raw-string literals count once per
//! physical line that contains a token (finalize-on-newline runs at the top of the loop
//! regardless of state, so each `\n` inside a raw string still resets
//! `has_code`).

use std::path::{Path, PathBuf};

use crate::approval::Exception;
use crate::scanner::ScanCursor;

pub(crate) fn code_lines(source: &str) -> usize {
    let mut cursor = ScanCursor::new(source.as_bytes());
    std::iter::from_fn(|| cursor.next_event())
        .filter(|counted| *counted)
        .count()
}

/// Soft review floors. There is no hard cap: any file at or above its floor
/// passes once `split-judge` (or the owner) confirms it does one job and a
/// `slugaudit-line-exception:` comment records why.
pub(crate) const TEST_FILE_EXCEPTION_FLOOR: usize = 501;
/// Production files at or above this length require an approved
/// `slugaudit-line-exception:` comment (after `split-judge` says `KEEP`).
pub(crate) const PRODUCTION_EXCEPTION_FLOOR: usize = 200;

/// How `main` reports a single file: within limits (optionally via an
/// approved exception) or needing a `split-judge` check / exception.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Pass,
    PassWithException { reason: String },
    FailNeedsException { floor: usize },
}

/// The gate's policy. Production files auto-pass below
/// [`PRODUCTION_EXCEPTION_FLOOR`] (200); test files (`*_tests.rs`, `tests.rs`)
/// auto-pass below [`TEST_FILE_EXCEPTION_FLOOR`] (501). Above the floor there
/// is no hard cap — any valid `slugaudit-line-exception:` passes.
pub(crate) fn verdict(code_lines: usize, exception: Option<Exception>, test_file: bool) -> Verdict {
    let floor = if test_file {
        TEST_FILE_EXCEPTION_FLOOR
    } else {
        PRODUCTION_EXCEPTION_FLOOR
    };
    if code_lines >= floor {
        return match exception {
            Some(exception) => Verdict::PassWithException {
                reason: exception.reason,
            },
            None => Verdict::FailNeedsException { floor },
        };
    }
    Verdict::Pass
}

pub(crate) fn walk_rs_files(src_root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut out = Vec::new();
    walk_rs(src_root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk_rs(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}
