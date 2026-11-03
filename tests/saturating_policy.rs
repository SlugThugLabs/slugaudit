#![forbid(unsafe_code)]

//! Guard for the saturating/wrapping policy.
//!
//! A saturating or wrapping operation hides an overflow by producing a
//! plausible number. The policy: every `saturating_*` / `wrapping_*` call and
//! every `Saturating<..>` / `Wrapping<..>` use in `src/` must carry a
//! `// KEEP saturating: <reason>` or `// KEEP wrapping: <reason>` comment
//! within the 3 lines above it, stating why the clamp or wrap is the intended
//! meaning. Anything else is converted to a checked operation that returns an
//! error.
//!
//! The scan reads every `.rs` file under `src/` with `std::fs` and looks only
//! at code: text inside string literals and comments is ignored.

use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::CharIndices;

/// How many lines above a use the marker may sit.
const MARKER_WINDOW_LINES: usize = 3;

/// The needles that start a flagged use, with the marker kind each needs.
const NEEDLES: [(&str, &str); 6] = [
    ("saturating_", "saturating"),
    ("wrapping_", "wrapping"),
    ("Saturating<", "saturating"),
    ("Wrapping<", "wrapping"),
    ("Saturating(", "saturating"),
    ("Wrapping(", "wrapping"),
];

/// One source line split into its code and its line comment.
#[derive(Debug, Default)]
struct Line {
    /// The line with comments and the insides of literals blanked out.
    code: String,
    /// The text of a `//` comment on this line, without the slashes.
    comment: Option<String>,
}

/// One flagged use.
#[derive(Debug, PartialEq, Eq)]
struct Finding {
    /// One-based line of the use.
    line: usize,
    /// The needle that matched.
    needle: &'static str,
    /// Why the use is not allowed.
    problem: &'static str,
}

/// Where the lexer is inside the text.
enum Mode {
    Code,
    LineComment,
    BlockComment { depth: usize },
    Str { escaped: bool },
    RawStr { hashes: usize },
    Char { escaped: bool },
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Consumes `count` more characters.
fn skip(chars: &mut Peekable<CharIndices<'_>>, count: usize) {
    for _ in 0..count {
        chars.next();
    }
}

/// The number of `#` characters starting `rest`, and whether a `"` follows
/// them: the opening of a raw string, seen after its `r`.
fn raw_string_open(rest: &str) -> Option<usize> {
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    rest.get(hashes..)?.starts_with('"').then_some(hashes)
}

/// Splits `text` into lines of code and line comments.
fn lex(text: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    let mut current = Line::default();
    let mut mode = Mode::Code;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let rest = text
            .get(index..)
            .ok_or_else(|| format!("byte {index} is not a character boundary"))?;
        if c == '\n' {
            if matches!(mode, Mode::LineComment) {
                mode = Mode::Code;
            }
            lines.push(std::mem::take(&mut current));
            continue;
        }
        match &mut mode {
            Mode::LineComment => {
                current.comment.get_or_insert_with(String::new).push(c);
            }
            Mode::BlockComment { depth } => {
                if rest.starts_with("*/") {
                    skip(&mut chars, 1);
                    if let Some(outer) = depth.checked_sub(1).filter(|outer| *outer > 0) {
                        *depth = outer;
                    } else {
                        mode = Mode::Code;
                    }
                } else if rest.starts_with("/*") {
                    skip(&mut chars, 1);
                    *depth = depth
                        .checked_add(1)
                        .ok_or_else(|| "block comment nesting overflowed".to_owned())?;
                }
                current.code.push(' ');
            }
            Mode::Str { escaped } => {
                if *escaped {
                    *escaped = false;
                } else if c == '\\' {
                    *escaped = true;
                } else if c == '"' {
                    mode = Mode::Code;
                }
                current.code.push(' ');
            }
            Mode::RawStr { hashes } => {
                let closing = format!("\"{}", "#".repeat(*hashes));
                if c == '"' && rest.starts_with(&closing) {
                    skip(&mut chars, *hashes);
                    mode = Mode::Code;
                }
                current.code.push(' ');
            }
            Mode::Char { escaped } => {
                if *escaped {
                    *escaped = false;
                } else if c == '\\' {
                    *escaped = true;
                } else if c == '\'' {
                    mode = Mode::Code;
                }
                current.code.push(' ');
            }
            Mode::Code => {
                if rest.starts_with("//") {
                    skip(&mut chars, 1);
                    current.comment = Some(String::new());
                    mode = Mode::LineComment;
                } else if rest.starts_with("/*") {
                    skip(&mut chars, 1);
                    mode = Mode::BlockComment { depth: 1 };
                    current.code.push(' ');
                } else if c == '"' {
                    mode = Mode::Str { escaped: false };
                    current.code.push(' ');
                } else if c == 'r' && starts_raw_string(&current.code) {
                    let after = rest.get(1..).ok_or("raw string prefix")?;
                    if let Some(hashes) = raw_string_open(after) {
                        skip(
                            &mut chars,
                            hashes.checked_add(1).ok_or("raw string hashes")?,
                        );
                        mode = Mode::RawStr { hashes };
                        current.code.push(' ');
                    } else {
                        current.code.push(c);
                    }
                } else if c == '\'' {
                    let mut ahead = rest.chars().skip(1);
                    match (ahead.next(), ahead.next()) {
                        // `'\n'`, `'\''`, `'\u{..}'`: runs to the closing quote.
                        (Some('\\'), _) => {
                            mode = Mode::Char { escaped: false };
                            current.code.push(' ');
                        }
                        // `'x'`, `'"'`: a one-character literal.
                        (Some(_), Some('\'')) => {
                            skip(&mut chars, 2);
                            current.code.push(' ');
                        }
                        // A lifetime or label.
                        _ => current.code.push(c),
                    }
                } else {
                    current.code.push(c);
                }
            }
        }
    }
    lines.push(current);
    Ok(lines)
}

/// Whether an `r` just read in code opens a raw string: it is not the tail of
/// an identifier, except for the `b` of a `br"` prefix.
fn starts_raw_string(code_before: &str) -> bool {
    let mut previous = code_before.chars().rev();
    match previous.next() {
        None => true,
        Some('b') => !previous.next().is_some_and(is_ident_char),
        Some(c) => !is_ident_char(c),
    }
}

/// The kind and non-empty reason of a `KEEP` marker comment, if `comment` is
/// one. `Err(kind)` is a marker whose reason is empty.
fn parse_marker(comment: &str) -> Option<(&'static str, bool)> {
    let text = comment.trim_start_matches(['/', '!']).trim();
    for kind in ["saturating", "wrapping"] {
        if let Some(reason) = text
            .strip_prefix("KEEP ")
            .and_then(|tail| tail.strip_prefix(kind))
            .and_then(|tail| tail.strip_prefix(':'))
        {
            return Some((kind, !reason.trim().is_empty()));
        }
    }
    None
}

/// Every flagged use in `text` that has no valid marker above it.
fn scan_source(text: &str) -> Result<Vec<Finding>, String> {
    let lines = lex(text)?;
    let mut findings = Vec::new();
    for ((index, line), number) in lines.iter().enumerate().zip(1_usize..) {
        for (needle, kind) in NEEDLES {
            for (at, _) in line.code.match_indices(needle) {
                let before = line.code.get(..at).ok_or("needle offset")?;
                if before.chars().next_back().is_some_and(is_ident_char) {
                    continue;
                }
                let mut problem = Some("no `// KEEP` marker within the 3 lines above");
                for above in lines.iter().take(index).rev().take(MARKER_WINDOW_LINES) {
                    let Some((marker_kind, has_reason)) =
                        above.comment.as_deref().and_then(parse_marker)
                    else {
                        continue;
                    };
                    if marker_kind != kind {
                        problem = Some("the marker above names the other kind");
                    } else if has_reason {
                        problem = None;
                        break;
                    } else {
                        problem = Some("the marker above has an empty reason");
                    }
                }
                if let Some(problem) = problem {
                    findings.push(Finding {
                        line: number,
                        needle,
                        problem,
                    });
                }
            }
        }
    }
    Ok(findings)
}

/// Every `.rs` file under `dir`, in path order.
fn rust_files(dir: &Path) -> TestResult<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).check("read a source directory")? {
            let path = entry.check("read a directory entry")?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn every_saturating_or_wrapping_use_in_src_carries_a_keep_marker() -> TestResult {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = rust_files(&root.join("src"))?;
    ensure(!files.is_empty(), "src/ holds .rs files to scan")?;
    let mut report = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).check("read a source file")?;
        let relative = file.strip_prefix(root).check("file is under the root")?;
        for finding in scan_source(&text).check("lex a source file")? {
            report.push(format!(
                "{}:{}: `{}`: {}",
                relative.display(),
                finding.line,
                finding.needle,
                finding.problem
            ));
        }
    }
    ensure(
        report.is_empty(),
        format!(
            "saturating/wrapping uses without a `// KEEP saturating: <reason>` or \
             `// KEEP wrapping: <reason>` marker:\n{}",
            report.join("\n")
        ),
    )
}

fn lines_of(text: &str) -> TestResult<Vec<(usize, &'static str)>> {
    Ok(scan_source(text)
        .check("lex the fixture")?
        .into_iter()
        .map(|finding| (finding.line, finding.needle))
        .collect())
}

#[test]
fn a_marked_use_passes() -> TestResult {
    let text = "fn f() {\n    // KEEP saturating: over budget is the answer\n    let x = 1_u8.saturating_add(1);\n}\n";
    ensure_eq(&lines_of(text)?, &Vec::new(), "no findings")
}

#[test]
fn a_marker_three_lines_above_passes_and_four_lines_above_fails() -> TestResult {
    let three = "// KEEP wrapping: hash mixing\n\n\nlet x = 1_u8.wrapping_add(1);\n";
    let four = "// KEEP wrapping: hash mixing\n\n\n\nlet x = 1_u8.wrapping_add(1);\n";
    ensure_eq(&lines_of(three)?, &Vec::new(), "three lines above")?;
    ensure_eq(
        &lines_of(four)?,
        &vec![(5, "wrapping_")],
        "four lines above",
    )
}

#[test]
fn an_unmarked_use_fails_with_its_line() -> TestResult {
    let text = "fn f() {\n    let x = 1_u8.saturating_add(1);\n}\n";
    ensure_eq(&lines_of(text)?, &vec![(2, "saturating_")], "one finding")
}

#[test]
fn a_marker_of_the_other_kind_or_with_no_reason_does_not_count() -> TestResult {
    let other = "// KEEP wrapping: not this kind\nlet x = 1_u8.saturating_add(1);\n";
    let empty = "// KEEP saturating:   \nlet x = 1_u8.saturating_add(1);\n";
    ensure_eq(&lines_of(other)?, &vec![(2, "saturating_")], "other kind")?;
    ensure_eq(&lines_of(empty)?, &vec![(2, "saturating_")], "empty reason")
}

#[test]
fn a_use_inside_a_string_literal_is_ignored() -> TestResult {
    let text = concat!(
        "let a = \"x.saturating_add(1)\";\n",
        "let b = \"multi\nline wrapping_mul\";\n",
        "let c = r#\"has \"quotes\" and Wrapping<u8>\"#;\n",
        "let d = b\"wrapping_add\";\n",
        "let e = '\"'; let f = \"escaped \\\" saturating_sub\";\n",
    );
    ensure_eq(&lines_of(text)?, &Vec::new(), "strings are not code")
}

#[test]
fn a_use_inside_a_comment_is_ignored() -> TestResult {
    let text = concat!(
        "// x.saturating_add(1)\n",
        "/// docs mention wrapping_mul\n",
        "/* block saturating_sub /* nested wrapping_add */ still comment Saturating<u8> */\n",
        "let ok = 1;\n",
    );
    ensure_eq(&lines_of(text)?, &Vec::new(), "comments are not code")
}

#[test]
fn code_after_a_literal_or_comment_on_the_same_line_is_still_scanned() -> TestResult {
    let text = "let a = \"s\"; let b = 1_u8.wrapping_add(1); // trailing saturating_add\n";
    ensure_eq(&lines_of(text)?, &vec![(1, "wrapping_")], "code is scanned")
}

#[test]
fn type_uses_and_path_calls_are_flagged_and_lookalike_names_are_not() -> TestResult {
    let text = concat!(
        "let a: Saturating<u8> = Saturating(1);\n",
        "let b = Wrapping(1u8);\n",
        "let c = usize::saturating_add(1, 2);\n",
        "let d = not_saturating_x + is_wrapping_y;\n",
        "let e: &'static str = \"\"; let f = 'a';\n",
    );
    ensure_eq(
        &lines_of(text)?,
        &vec![
            (1, "Saturating<"),
            (1, "Saturating("),
            (2, "Wrapping("),
            (3, "saturating_"),
        ],
        "findings",
    )
}
