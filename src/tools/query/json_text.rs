//! Serialized JSON text byte-cost measurement and prefixing for `query` cell clipping.

fn json_char_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        control if control < ' ' => 6,
        other => other.len_utf8(),
    }
}

fn walk_json_text(
    text: &str,
    budget: Option<usize>,
    mut out: Option<&mut String>,
) -> Result<(usize, usize), &'static str> {
    let mut used = 0usize;
    let mut chars = 0usize;
    for ch in text.chars() {
        let next = used
            .checked_add(json_char_len(ch))
            .ok_or("measuring a text cell")?;
        if let Some(cap) = budget
            && used > 0
            && next > cap
        {
            break;
        }
        used = next;
        chars = chars.checked_add(1).ok_or("counting text characters")?;
        if let Some(buf) = out.as_deref_mut() {
            buf.push(ch);
        }
    }
    Ok((used, chars))
}

/// Returns `(serialized_json_bytes, char_count)` for `text`.
pub(super) fn text_size(text: &str) -> Result<(usize, usize), &'static str> {
    walk_json_text(text, None, None)
}

/// Returns `(prefix, char_count)` fitting within `budget` serialized JSON bytes
/// (always keeping at least one character if `text` is non-empty).
pub(super) fn text_prefix(text: &str, budget: usize) -> Result<(String, usize), &'static str> {
    let mut kept = String::new();
    let (_, chars) = walk_json_text(text, Some(budget), Some(&mut kept))?;
    Ok((kept, chars))
}
