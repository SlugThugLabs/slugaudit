//! Rust `use`-tree parser: strips comments and visibility, then expands
//! nested brace trees (`use crate::{a, b::*, foo::{self, bar as x}};`)
//! into deduplicated import targets without truncating at `{`.

use crate::util::strip_comments;

/// Maximum brace nesting depth allowed when expanding a Rust `use` tree.
const MAX_USE_TREE_DEPTH: usize = 32;

/// Parses a raw Rust `use` statement into expanded targets.
///
/// Returns `None` for non-`use` items (such as `extern crate`) and malformed
/// `use` trees, `Some(vec![])` for valid empty-brace `use crate::foo::{};`,
/// and `Some(targets)` for valid `use` trees.
pub(super) fn parse_rust_import(raw: &str) -> Option<Vec<String>> {
    let cleaned = strip_comments(raw)?;
    let body = strip_use_keyword(strip_visibility(cleaned.trim()))?;
    let mut cursor = (!body.is_empty()).then_some(StrCursor(body))?;
    let mut targets = Vec::new();
    parse_tree(&mut cursor, "", 0, true, &mut targets)?;
    cursor.0.is_empty().then_some(targets)
}

fn strip_visibility(text: &str) -> &str {
    let Some(after_pub) = text
        .strip_prefix("pub")
        .filter(|rest| rest.starts_with(char::is_whitespace) || rest.starts_with('('))
    else {
        return text;
    };
    let trimmed = after_pub.trim_start();
    let Some(after_open) = trimmed.strip_prefix('(') else {
        return trimmed;
    };
    let mut depth: usize = 1;
    for (idx, ch) in after_open.char_indices() {
        match ch {
            '(' => {
                if let Some(next_depth) = depth.checked_add(1) {
                    depth = next_depth;
                }
            }
            ')' => {
                if let Some(next_depth) = depth.checked_sub(1) {
                    depth = next_depth;
                }
                if depth == 0 {
                    let next_idx = match idx.checked_add(1) {
                        Some(offset) => offset,
                        None => after_open.len(),
                    };
                    return after_open.get(next_idx..).map_or(text, str::trim_start);
                }
            }
            _ => {}
        }
    }
    text
}

fn strip_use_keyword(text: &str) -> Option<&str> {
    let after = text.strip_prefix("use").filter(|rest| {
        rest.starts_with(char::is_whitespace) || rest.starts_with('{') || rest.starts_with("::")
    })?;
    let trimmed = after.trim();
    let body = match trimmed.strip_suffix(';') {
        Some(without_semi) => without_semi.trim(),
        None => trimmed,
    };
    Some(body)
}

struct StrCursor<'a>(&'a str);

impl<'a> StrCursor<'a> {
    fn strip(&mut self, prefix: &str) -> bool {
        self.advance_if(prefix, false)
    }

    fn advance_if(&mut self, prefix: &str, require_whitespace: bool) -> bool {
        let Some(rest) = self.0.strip_prefix(prefix) else {
            return false;
        };
        if require_whitespace && !rest.starts_with(char::is_whitespace) {
            return false;
        }
        self.0 = rest.trim_start();
        true
    }

    fn next_non_as_ident(&mut self) -> Option<&'a str> {
        let ident = self.next_ident()?;
        (ident != "as").then_some(ident)
    }

    fn next_ident(&mut self) -> Option<&'a str> {
        let (prefix_len, body) = match self.0.strip_prefix("r#") {
            Some(after_raw) => (2usize, after_raw),
            None => (0usize, self.0),
        };
        let mut chars = body.char_indices();
        let (_, first) = chars.next()?;
        if !first.is_alphabetic() && first != '_' {
            return None;
        }
        let mut body_len = first.len_utf8();
        for (idx, ch) in chars {
            if !ch.is_alphanumeric() && ch != '_' {
                break;
            }
            body_len = idx.checked_add(ch.len_utf8())?;
        }
        let len = prefix_len.checked_add(body_len)?;
        let word = self.0.get(..len)?;
        self.0 = self.0.get(len..)?.trim_start();
        Some(word)
    }
}

fn parse_tree(
    cursor: &mut StrCursor<'_>,
    prefix: &str,
    depth: usize,
    allow_leading_colon: bool,
    out: &mut Vec<String>,
) -> Option<()> {
    if allow_leading_colon && prefix.is_empty() {
        cursor.strip("::");
    }
    if cursor.strip("*") {
        return (!prefix.is_empty()).then(|| push_unique(out, join_segment(prefix, "*")));
    }
    if cursor.strip("{") {
        let next_depth = depth.checked_add(1)?;
        if next_depth > MAX_USE_TREE_DEPTH {
            return None;
        }
        return parse_tree_list(cursor, prefix, next_depth, out);
    }
    let ident = cursor.next_non_as_ident()?;
    let next_prefix = if ident == "self" && !prefix.is_empty() {
        prefix.to_owned()
    } else {
        join_segment(prefix, ident)
    };
    if cursor.strip("::") {
        return parse_tree(cursor, &next_prefix, depth, false, out);
    }
    if cursor.advance_if("as", true) {
        cursor.next_non_as_ident()?;
    }
    push_unique(out, next_prefix);
    Some(())
}

fn parse_tree_list(
    cursor: &mut StrCursor<'_>,
    prefix: &str,
    depth: usize,
    out: &mut Vec<String>,
) -> Option<()> {
    while !cursor.0.is_empty() {
        if cursor.strip("}") {
            return Some(());
        }
        if !cursor.strip(",") {
            parse_tree(cursor, prefix, depth, prefix.is_empty(), out)?;
            if !cursor.strip(",") && !cursor.0.starts_with('}') {
                return None;
            }
        }
    }
    None
}

fn join_segment(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_owned()
    } else {
        format!("{prefix}::{segment}")
    }
}

fn push_unique(out: &mut Vec<String>, specifier: String) {
    if !out.contains(&specifier) {
        out.push(specifier);
    }
}

#[cfg(test)]
#[path = "rust_use_tree_tests.rs"]
mod tests;
