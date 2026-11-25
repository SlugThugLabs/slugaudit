//! Pure-text parser for Rust `#[path = "..."]` and `#[cfg_attr(..., path = "...")]`
//! outer attributes and Rust string literals.

use crate::util::skip_ws_and_comments;

const MAX_ATTR_DEPTH: u8 = 32;

/// Extracts the string path from a Rust outer `#[path = "..."]` or
/// `#[cfg_attr(..., path = "...")]` attribute item.
pub(super) fn parse_attribute_item(text: &str) -> Option<String> {
    let after_hash = text.trim().strip_prefix('#')?.trim_start();
    let body = after_hash.strip_prefix('[')?.strip_suffix(']')?;
    extract_path_attr(body, 0)
}

fn extract_path_attr(body: &str, depth: u8) -> Option<String> {
    if depth >= MAX_ATTR_DEPTH {
        return None;
    }
    let rest = skip_ws_and_comments(body)?;
    let ident_end = rest
        .find(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .map_or(rest.len(), |idx| idx);
    let ident = rest.get(..ident_end)?;
    let after_ident = skip_ws_and_comments(rest.get(ident_end..)?)?;
    match ident {
        "path" => {
            let after_eq = after_ident.strip_prefix('=')?;
            let (value, trailing) = parse_rust_string_literal(after_eq)?;
            skip_ws_and_comments(trailing)?.is_empty().then_some(value)
        }
        "cfg_attr" => parse_cfg_attr(after_ident.strip_prefix('(')?, depth.checked_add(1)?),
        _ => None,
    }
}

fn parse_cfg_attr(input: &str, depth: u8) -> Option<String> {
    let mut found = None;
    let mut parens = 0usize;
    let mut arg_start = 0usize;
    let mut is_predicate = true;
    let mut rest = input;

    while !rest.is_empty() {
        let current = skip_ws_and_comments(rest)?;
        if current.starts_with('"') || raw_string_prefix(current).is_some() {
            rest = parse_rust_string_literal(current)?.1;
            continue;
        }
        let pos = input.len().checked_sub(current.len())?;
        let mut chars = current.chars();
        let ch = chars.next()?;
        rest = chars.as_str();
        match ch {
            '(' | '[' | '{' => parens = parens.checked_add(1)?,
            ')' | ']' | '}' if parens > 0 => parens = parens.checked_sub(1)?,
            ')' | ',' if ch == ')' || parens == 0 => {
                if !is_predicate
                    && let Some(path) = extract_path_attr(input.get(arg_start..pos)?, depth)
                {
                    found = Some(path);
                }
                if ch == ')' {
                    return skip_ws_and_comments(rest)?.is_empty().then_some(found)?;
                }
                is_predicate = false;
                arg_start = input.len().checked_sub(rest.len())?;
            }
            ']' | '}' => return None,
            _ => {}
        }
    }
    None
}

/// If `text` begins with a Rust raw string prefix `r#*"`, returns the `#`
/// count and the slice immediately following the opening `"`.
fn raw_string_prefix(text: &str) -> Option<(usize, &str)> {
    let after_r = text.strip_prefix('r')?;
    let hashes = after_r.bytes().take_while(|byte| *byte == b'#').count();
    let body = after_r.get(hashes..)?.strip_prefix('"')?;
    Some((hashes, body))
}

fn parse_rust_string_literal(input: &str) -> Option<(String, &str)> {
    let rest = skip_ws_and_comments(input)?;
    if let Some((hashes, body)) = raw_string_prefix(rest) {
        let closer = format!("\"{}", "#".repeat(hashes));
        let (raw, tail) = body.split_once(&closer)?;
        return Some((raw.to_owned(), tail));
    }
    let mut out = String::new();
    let mut chars = rest.strip_prefix('"')?.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            return Some((out, chars.as_str()));
        }
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
    None
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
