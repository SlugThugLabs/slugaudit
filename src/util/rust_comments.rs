//! Shared Rust comment-lexing primitives used by attribute and `use`-tree
//! parsers so line and nested block comments have one implementation.

/// If `input` starts with a `//` line comment, returns the slice after the
/// comment and its terminating newline (if any), or `None` if `input` does
/// not start with `//`.
pub(crate) fn skip_line_comment(input: &str) -> Option<&str> {
    let after_slashes = input.strip_prefix("//")?;
    Some(after_slashes.split_once('\n').map_or("", |(_, rest)| rest))
}

/// If `input` starts with a `/* ... */` block comment, skips it (including
/// nested `/* ... */` pairs) and returns the remaining slice, or `None` if
/// `input` does not start with `/*`, is unterminated, or overflows depth.
pub(crate) fn skip_block_comment(input: &str) -> Option<&str> {
    let mut rest = input.strip_prefix("/*")?;
    let mut depth: usize = 1;
    while depth > 0 {
        if let Some(after) = rest.strip_prefix("/*") {
            depth = depth.checked_add(1)?;
            rest = after;
        } else if let Some(after) = rest.strip_prefix("*/") {
            depth = depth.checked_sub(1)?;
            rest = after;
        } else {
            let mut chars = rest.chars();
            chars.next()?;
            rest = chars.as_str();
        }
    }
    Some(rest)
}

/// Advances `input` past leading whitespace, `//...` line comments, and nested
/// `/* ... */` block comments. Returns `None` if a block comment is
/// unterminated.
pub(crate) fn skip_ws_and_comments(mut input: &str) -> Option<&str> {
    loop {
        input = input.trim_start();
        if let Some(after_line) = skip_line_comment(input) {
            input = after_line;
        } else if input.starts_with("/*") {
            input = skip_block_comment(input)?;
        } else {
            return Some(input);
        }
    }
}

/// Strips `//...` line comments and nested `/* ... */` block comments from
/// `input`, replacing each comment span with a single ASCII space. Returns
/// `None` if a block comment is unterminated.
pub(crate) fn strip_comments(mut input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    while !input.is_empty() {
        if let Some(after_line) = skip_line_comment(input) {
            out.push(' ');
            input = after_line;
        } else if input.starts_with("/*") {
            input = skip_block_comment(input)?;
            out.push(' ');
        } else {
            let mut chars = input.chars();
            let ch = chars.next()?;
            out.push(ch);
            input = chars.as_str();
        }
    }
    Some(out)
}
