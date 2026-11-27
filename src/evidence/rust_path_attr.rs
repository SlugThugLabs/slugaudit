//! Direct `tree-sitter-rust` CST extractor for outer `#[path = "..."]` and
//! nested `#[cfg_attr(..., path = "...")]` attributes and Rust string literals.

use super::named_children;
use tree_sitter::Node;

const MAX_ATTR_DEPTH: u8 = 32;

/// Extracts the string path from a Rust outer `#[path = "..."]` or
/// `#[cfg_attr(..., path = "...")]` `attribute_item` CST node.
pub(super) fn parse_attribute_node(attr_item: Node<'_>, source: &str) -> Option<String> {
    if attr_item.kind() != "attribute_item" {
        return None;
    }
    let attr = named_children(attr_item).find(|child| child.kind() == "attribute")?;
    let head = named_children(attr).next()?;
    if head.kind() != "identifier" {
        return None;
    }
    let name = source.get(head.start_byte()..head.end_byte())?;
    match name {
        "path" => {
            if attr.child_by_field_name("arguments").is_some() {
                return None;
            }
            let value = attr.child_by_field_name("value")?;
            parse_string_literal_node(value, source)
        }
        "cfg_attr" => {
            if attr.child_by_field_name("value").is_some() {
                return None;
            }
            let args = attr.child_by_field_name("arguments")?;
            extract_cfg_attr_path(args, source)
        }
        _ => None,
    }
}

fn extract_cfg_attr_path(root_tt: Node<'_>, source: &str) -> Option<String> {
    let mut found = None;
    let mut stack: Vec<(Vec<Node<'_>>, u8)> = Vec::new();
    push_cfg_attr_segments(root_tt, 1, &mut stack)?;

    while let Some((segment, depth)) = stack.pop() {
        match segment.as_slice() {
            [ident, eq, lit]
                if ident.is_named()
                    && ident.kind() == "identifier"
                    && source.get(ident.start_byte()..ident.end_byte()) == Some("path")
                    && !eq.is_named()
                    && eq.kind() == "=" =>
            {
                if let Some(value) = parse_string_literal_node(*lit, source) {
                    found = Some(value);
                }
            }
            [ident, inner_tt]
                if ident.is_named()
                    && ident.kind() == "identifier"
                    && source.get(ident.start_byte()..ident.end_byte()) == Some("cfg_attr")
                    && inner_tt.is_named()
                    && inner_tt.kind() == "token_tree" =>
            {
                let next_depth = depth.checked_add(1)?;
                if next_depth > MAX_ATTR_DEPTH {
                    return None;
                }
                push_cfg_attr_segments(*inner_tt, next_depth, &mut stack)?;
            }
            _ => {}
        }
    }
    found
}

/// Splits a parenthesized `cfg_attr(...)` `token_tree` on top-level `,` tokens,
/// discards segment `0` (the `cfg` predicate), and pushes the remaining
/// attribute segments onto `stack` in reverse so popping visits them in source
/// order.
fn push_cfg_attr_segments<'tree>(
    tt: Node<'tree>,
    depth: u8,
    stack: &mut Vec<(Vec<Node<'tree>>, u8)>,
) -> Option<()> {
    if tt.kind() != "token_tree" {
        return None;
    }
    let mut tokens = Vec::new();
    let mut cursor = tt.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if !matches!(child.kind(), "line_comment" | "block_comment") {
                tokens.push(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    let (open, rest) = tokens.split_first()?;
    let (close, interior) = rest.split_last()?;
    if open.is_named() || open.kind() != "(" || close.is_named() || close.kind() != ")" {
        return None;
    }

    let mut segments: Vec<Vec<Node<'tree>>> = Vec::new();
    let mut current: Vec<Node<'tree>> = Vec::new();
    for &tok in interior {
        if !tok.is_named() && tok.kind() == "," {
            segments.push(std::mem::take(&mut current));
        } else {
            current.push(tok);
        }
    }
    segments.push(current);

    if segments.len() < 2 {
        return Some(());
    }
    for seg in segments.into_iter().skip(1).rev() {
        if !seg.is_empty() {
            stack.push((seg, depth));
        }
    }
    Some(())
}

fn parse_string_literal_node(lit: Node<'_>, source: &str) -> Option<String> {
    match lit.kind() {
        "raw_string_literal" => {
            if let Some(content) = named_children(lit).find(|c| c.kind() == "string_content") {
                return source
                    .get(content.start_byte()..content.end_byte())
                    .map(str::to_owned);
            }
            let raw = source.get(lit.start_byte()..lit.end_byte())?;
            (raw.starts_with('r') && raw.contains('"')).then(String::new)
        }
        "string_literal" => {
            let raw = source.get(lit.start_byte()..lit.end_byte())?;
            let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
            unescape_cooked_str(inner)
        }
        _ => None,
    }
}

fn unescape_cooked_str(inner: &str) -> Option<String> {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let esc = chars.next()?;
        match esc {
            '\\' | '"' | '\'' => out.push(esc),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            'x' | 'u' => out.push(parse_hex_escape(esc, &mut chars)?),
            '\n' | '\r' => {
                let tail = chars.as_str();
                let after_nl = if esc == '\r' {
                    tail.strip_prefix('\n')?
                } else {
                    tail
                };
                chars = after_nl.trim_start_matches([' ', '\t']).chars();
            }
            _ => return None,
        }
    }
    Some(out)
}

fn parse_hex_escape(esc: char, chars: &mut std::str::Chars<'_>) -> Option<char> {
    let ((hex, tail), max_scalar, is_unicode) = if esc == 'x' {
        (chars.as_str().split_at_checked(2)?, 0x7F, false)
    } else {
        (
            chars.as_str().strip_prefix('{')?.split_once('}')?,
            0x10_FFFF,
            true,
        )
    };
    *chars = tail.chars();
    let mut code = 0u32;
    let mut digits = 0usize;
    for ch in hex.chars() {
        if is_unicode && ch == '_' {
            continue;
        }
        let digit = ch.to_digit(16)?;
        digits = digits.checked_add(1)?;
        if is_unicode && digits > 6 {
            return None;
        }
        code = code.checked_mul(16)?.checked_add(digit)?;
    }
    if digits == 0 || code > max_scalar {
        return None;
    }
    char::from_u32(code)
}
