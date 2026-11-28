//! Quote-aware JSONC (`*.jsonc`) comment detection and normalization.
//!
//! Applied strictly to `.jsonc` paths (such as `OpenCode`'s `opencode.jsonc`),
//! leaving `.json` files strict as required by client config validation.
//! Read-only detection (`has_slugaudit`) normalizes comments in memory; config
//! writers check [`has_jsonc_comments`] first and refuse to overwrite commented
//! `.jsonc` files rather than erasing user comments.

use std::path::Path;

/// Whether `path` has a `.jsonc` extension, opting into JSON-with-comments and
/// trailing-comma normalization while keeping `.json` files strict.
pub(super) fn is_jsonc_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonc"))
}

/// Returns `true` when `input` contains any `//` line comment or `/*` block
/// comment outside JSON string literals.
#[must_use]
pub(super) fn has_jsonc_comments(input: &str) -> bool {
    let mut offset = 0_usize;
    while let Some(tail) = input.get(offset..) {
        let Some(ch) = tail.chars().next() else {
            break;
        };
        if ch == '"' {
            let Some(after_open) = tail.get(1..) else {
                break;
            };
            match find_string_end(after_open) {
                Some(len) => {
                    let Some(next) = offset.checked_add(1).and_then(|n| n.checked_add(len)) else {
                        break;
                    };
                    offset = next;
                    continue;
                }
                None => break,
            }
        }
        if tail.starts_with("//") || tail.starts_with("/*") {
            return true;
        }
        let Some(next) = offset.checked_add(ch.len_utf8()) else {
            break;
        };
        offset = next;
    }
    false
}

/// Replaces `//` line comments and closed `/* ... */` block comments outside
/// JSON string literals with spaces (preserving `\r` and `\n` for line/column
/// error reporting), then strips trailing commas before `}` or `]`.
#[must_use]
pub(super) fn strip_jsonc_comments_and_trailing_commas(input: &str) -> String {
    let without_comments = strip_jsonc_comments(input);
    strip_jsonc_trailing_commas(&without_comments)
}

/// Finds the byte index right after the unescaped closing `"` in `after_open`
/// (which starts immediately after an opening `"`), or `None` if unclosed.
fn find_string_end(after_open: &str) -> Option<usize> {
    let mut escaped = false;
    for (idx, ch) in after_open.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return idx.checked_add(1);
        }
    }
    None
}

/// Finds the byte length of `body + "*/"` starting right after `/*`.
/// A JSONC block comment always terminates at the very first `*/`.
fn find_block_comment_end(after_open: &str) -> Option<usize> {
    after_open.find("*/").and_then(|idx| idx.checked_add(2))
}

fn push_blanked(out: &mut String, span: &str) {
    for ch in span.chars() {
        out.push(if matches!(ch, '\n' | '\r') { ch } else { ' ' });
    }
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut offset = 0_usize;

    while let Some(tail) = input.get(offset..) {
        let Some(ch) = tail.chars().next() else {
            break;
        };
        if ch == '"' {
            let after_open = tail.get(1..);
            let span_len = match after_open.and_then(find_string_end) {
                Some(end) => 1_usize.checked_add(end),
                None => Some(tail.len()),
            };
            let Some(len) = span_len else { break };
            if let Some(literal) = tail.get(..len) {
                out.push_str(literal);
            }
            let Some(next) = offset.checked_add(len) else {
                break;
            };
            offset = next;
            continue;
        }
        if tail.starts_with("//") {
            let line_len = match tail.find('\n') {
                Some(nl) => nl.checked_add(1),
                None => Some(tail.len()),
            };
            let Some(len) = line_len else { break };
            if let Some(comment) = tail.get(..len) {
                push_blanked(&mut out, comment);
            }
            let Some(next) = offset.checked_add(len) else {
                break;
            };
            offset = next;
            continue;
        }
        if let Some(after_open) = tail.strip_prefix("/*")
            && let Some(body_and_close_len) = find_block_comment_end(after_open)
            && let Some(total_len) = 2_usize.checked_add(body_and_close_len)
        {
            if let Some(comment) = tail.get(..total_len) {
                push_blanked(&mut out, comment);
            }
            let Some(next) = offset.checked_add(total_len) else {
                break;
            };
            offset = next;
            continue;
        }
        out.push(ch);
        let Some(next) = offset.checked_add(ch.len_utf8()) else {
            break;
        };
        offset = next;
    }
    out
}

/// Copies a JSON string literal starting after its opening `"`, preserving all
/// escaped characters until its unescaped closing `"` (or end of input).
fn consume_string_literal(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    out: &mut String,
) {
    let mut escaped = false;
    for (_, ch) in chars.by_ref() {
        out.push(ch);
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            break;
        }
    }
}

fn strip_jsonc_trailing_commas(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    let mut prev_token: Option<char> = None;

    while let Some((idx, ch)) = chars.next() {
        if ch == '"' {
            out.push(ch);
            consume_string_literal(&mut chars, &mut out);
            prev_token = Some('"');
            continue;
        }
        if ch == ',' {
            let follows_value = !matches!(prev_token, None | Some('{' | '[' | ':' | ','));
            let next_non_ws = input
                .get(idx..)
                .and_then(|rest| rest.strip_prefix(','))
                .and_then(|rest| rest.chars().find(|c| !c.is_ascii_whitespace()));
            if follows_value && matches!(next_non_ws, Some('}' | ']')) {
                out.push(' ');
                continue;
            }
        }
        if !ch.is_ascii_whitespace() {
            prev_token = Some(ch);
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
#[path = "jsonc_tests.rs"]
mod tests;
