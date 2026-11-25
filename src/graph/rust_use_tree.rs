//! Rust `use`-tree parser: strips comments and visibility, then expands
//! nested brace trees (`use crate::{a, b::*, foo::{self, bar as x}};`)
//! into deduplicated import targets without truncating at `{`.

use crate::util::strip_comments;
use std::collections::HashSet;

/// Maximum brace nesting depth allowed when expanding a Rust `use` tree.
const MAX_USE_TREE_DEPTH: usize = 32;

/// Maximum `::`-separated path segments allowed in any expanded `use` path.
const MAX_PATH_SEGMENTS: usize = 64;

/// Maximum number of expanded targets allowed from a single `use` statement.
const MAX_EXPANDED_TARGETS: usize = 256;

/// Parses a raw Rust `use` statement into expanded targets.
///
/// Returns `None` for non-`use` items (such as `extern crate`), malformed
/// `use` trees, or trees exceeding expansion bounds; `Some(vec![])` for valid
/// empty-brace `use crate::foo::{};`; and `Some(targets)` for valid `use`
/// trees.
pub(super) fn parse_rust_import(raw: &str) -> Option<Vec<String>> {
    let cleaned = strip_comments(raw)?;
    let body = strip_use_keyword(strip_visibility(cleaned.trim()))?;
    let mut cursor = (!body.is_empty()).then_some(StrCursor(body))?;
    let mut state = ExpansionState::default();
    parse_tree(&mut cursor, &mut state, 0, true)?;
    cursor.0.is_empty().then_some(state.targets)
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
    let Some(rest) = after_balanced_parens(after_open) else {
        return text;
    };
    rest.trim_start()
}

fn after_balanced_parens(after_open: &str) -> Option<&str> {
    let mut depth: usize = 1;
    for (idx, ch) in after_open.char_indices() {
        if ch == '(' {
            depth = depth.checked_add(1)?;
        } else if ch == ')' {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return after_open.get(idx.checked_add(1)?..);
            }
        }
    }
    None
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

#[derive(Default)]
struct ExpansionState {
    prefix: String,
    segments: usize,
    targets: Vec<String>,
    seen: HashSet<String>,
}

impl ExpansionState {
    fn bump_segment_count(&mut self) -> Option<()> {
        let next = self
            .segments
            .checked_add(1)
            .filter(|&n| n <= MAX_PATH_SEGMENTS)?;
        self.segments = next;
        Some(())
    }

    fn append_segment(&mut self, segment: &str) -> Option<()> {
        let needs_separator = self.segments > 0;
        self.bump_segment_count()?;
        if needs_separator {
            self.prefix.push_str("::");
        }
        self.prefix.push_str(segment);
        Some(())
    }

    fn push_unique(&mut self) -> Option<()> {
        if self.seen.contains(&self.prefix) {
            return Some(());
        }
        (self.targets.len() < MAX_EXPANDED_TARGETS).then_some(())?;
        // KEEP clone: prefix buffer is mutated in place across sibling branches
        let specifier = self.prefix.clone();
        // KEEP clone: HashSet deduplicates in O(1) while Vec preserves source order
        self.seen.insert(specifier.clone());
        self.targets.push(specifier);
        Some(())
    }
}

fn parse_tree(
    cursor: &mut StrCursor<'_>,
    state: &mut ExpansionState,
    depth: usize,
    allow_leading_colon: bool,
) -> Option<()> {
    let (saved_len, saved_segments) = (state.prefix.len(), state.segments);
    let result = parse_tree_segments(cursor, state, depth, allow_leading_colon);
    state.prefix.truncate(saved_len);
    state.segments = saved_segments;
    result
}

fn parse_tree_segments(
    cursor: &mut StrCursor<'_>,
    state: &mut ExpansionState,
    depth: usize,
    mut allow_leading_colon: bool,
) -> Option<()> {
    loop {
        if allow_leading_colon && state.prefix.is_empty() && cursor.strip("::") {
            state.prefix.push_str("::");
        }
        allow_leading_colon = false;
        if cursor.strip("*") {
            (state.segments > 0).then_some(())?;
            state.append_segment("*")?;
            return state.push_unique();
        }
        if cursor.strip("{") {
            let next_depth = depth.checked_add(1).filter(|&d| d <= MAX_USE_TREE_DEPTH)?;
            return parse_tree_list(cursor, state, next_depth);
        }
        let ident = cursor.next_non_as_ident()?;
        if ident == "self" && state.segments > 0 {
            state.bump_segment_count()?;
        } else {
            state.append_segment(ident)?;
        }
        if cursor.strip("::") {
            continue;
        }
        if cursor.advance_if("as", true) {
            cursor.next_non_as_ident()?;
        }
        return state.push_unique();
    }
}

fn parse_tree_list(
    cursor: &mut StrCursor<'_>,
    state: &mut ExpansionState,
    depth: usize,
) -> Option<()> {
    while !cursor.0.is_empty() {
        if cursor.strip("}") {
            return Some(());
        }
        if !cursor.strip(",") {
            parse_tree(cursor, state, depth, state.prefix.is_empty())?;
            (cursor.strip(",") || cursor.0.starts_with('}')).then_some(())?;
        }
    }
    None
}

#[cfg(test)]
#[path = "rust_use_tree_tests.rs"]
mod tests;
