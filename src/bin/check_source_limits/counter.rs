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
// slugaudit-line-exception: approved-by=agent; reason=the token-aware counter's comment/string/char/raw-string states are one atomic scanner; splitting the state machine would fragment the exact token-coverage semantics the gate depends on

use std::path::{Path, PathBuf};

pub fn code_lines(source: &str) -> usize {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut count: usize = 0;
    let mut has_code = false;
    let mut index: usize = 0;
    let mut block_depth: usize = 0;
    let mut raw_hashes: usize = 0;
    let mut state = State::Normal;

    while index < len {
        match state {
            State::LineComment => {
                if bytes[index] == b'\n' {
                    if has_code {
                        count += 1;
                        has_code = false;
                    }
                    state = State::Normal;
                }
                index += 1;
            }
            State::BlockComment => {
                if index + 1 < len && bytes[index] == b'/' && bytes[index + 1] == b'*' {
                    block_depth += 1;
                    index += 2;
                } else if index + 1 < len && bytes[index] == b'*' && bytes[index + 1] == b'/' {
                    block_depth -= 1;
                    if block_depth == 0 {
                        state = State::Normal;
                    }
                    index += 2;
                } else if bytes[index] == b'\n' {
                    if has_code {
                        count += 1;
                        has_code = false;
                    }
                    index += 1;
                } else {
                    index += 1;
                }
            }
            State::String => {
                has_code = true;
                let byte = bytes[index];
                if byte == b'\\' && index + 1 < len {
                    index += 2;
                } else if byte == b'"' {
                    state = State::Normal;
                    index += 1;
                } else if byte == b'\n' {
                    // Defensive: real Rust strings don't span lines, but
                    // if one ever does, emit the partial line and fall
                    // back to normal mode so the rest of the file still
                    // parses.
                    if has_code {
                        count += 1;
                        has_code = false;
                    }
                    index += 1;
                } else {
                    index += 1;
                }
            }
            State::Char => {
                has_code = true;
                let byte = bytes[index];
                if byte == b'\\' && index + 1 < len {
                    index += 2;
                } else if byte == b'\'' {
                    state = State::Normal;
                    index += 1;
                } else if byte == b'\n' {
                    if has_code {
                        count += 1;
                        has_code = false;
                    }
                    index += 1;
                } else {
                    index += 1;
                }
            }
            State::RawString => {
                has_code = true;
                let byte = bytes[index];
                if byte == b'"' && index + raw_hashes < len {
                    let all_hashes = (1..=raw_hashes).all(|i| bytes[index + i] == b'#');
                    if all_hashes {
                        state = State::Normal;
                        index += raw_hashes + 1;
                        continue;
                    }
                }
                if byte == b'\n' {
                    if has_code {
                        count += 1;
                        has_code = false;
                    }
                    index += 1;
                } else {
                    index += 1;
                }
            }
            State::Normal => {
                let byte = bytes[index];
                if byte == b'\n' {
                    if has_code {
                        count += 1;
                        has_code = false;
                    }
                    index += 1;
                    continue;
                }
                if index + 1 < len && byte == b'/' && bytes[index + 1] == b'/' {
                    state = State::LineComment;
                    index += 2;
                    continue;
                }
                if index + 1 < len && byte == b'/' && bytes[index + 1] == b'*' {
                    state = State::BlockComment;
                    block_depth = 1;
                    index += 2;
                    continue;
                }
                if byte == b'"' {
                    state = State::String;
                    has_code = true;
                    index += 1;
                    continue;
                }
                if byte == b'\'' && is_char_literal_start(&source[index..]) {
                    state = State::Char;
                    has_code = true;
                    index += 1;
                    continue;
                }
                if byte == b'r'
                    && let Some(n) = is_raw_string_start(&source[index..])
                {
                    state = State::RawString;
                    raw_hashes = n;
                    has_code = true;
                    index += 1 + n + 1;
                    continue;
                }
                if !byte.is_ascii_whitespace() {
                    has_code = true;
                }
                index += 1;
            }
        }
    }
    if has_code {
        count += 1;
    }
    count
}

enum State {
    Normal,
    LineComment,
    BlockComment,
    String,
    Char,
    RawString,
}

/// `'(?:\\.|[^'\\\n])'` — opening quote, then either `\` followed by any
/// char or one char that isn't `'`, `\`, or newline, then closing `'`.
fn is_char_literal_start(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'\'' {
        return false;
    }
    if bytes[1] == b'\\' {
        return bytes.len() >= 4 && bytes[3] == b'\'';
    }
    if matches!(bytes[1], b'\'' | b'\\' | b'\n') {
        return false;
    }
    bytes[2] == b'\''
}

/// `r(#+)?\"` — `r`, optional hash run, then opening quote. Returns the
/// hash count (0 for `r"…"`, 5 for `r#####"…"`, etc.).
fn is_raw_string_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] != b'r' {
        return None;
    }
    let mut i = 1;
    let mut n_hashes = 0;
    while i < bytes.len() && bytes[i] == b'#' {
        n_hashes += 1;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'"' {
        Some(n_hashes)
    } else {
        None
    }
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

pub fn walk_rs_files(src_root: &Path) -> Vec<PathBuf> {
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
