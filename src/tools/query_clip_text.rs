//! Measuring and cutting text by the bytes it takes serialized as JSON.

use super::room_error;
use rmcp::ErrorData;

/// Serialized JSON bytes a character takes inside a string.
fn json_char_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        control if control < ' ' => 6,
        other => other.len_utf8(),
    }
}

/// `(serialized content bytes, characters)` of a whole text.
pub(super) fn text_size(text: &str) -> Result<(usize, usize), ErrorData> {
    let cost = text.chars().try_fold(0usize, |cost, character| {
        cost.checked_add(json_char_len(character))
    });
    let cost = cost.ok_or_else(|| room_error("measuring a text cell"))?;
    Ok((cost, text.chars().count()))
}

/// The longest prefix whose serialized content fits `budget` (at least one
/// character), and its length in characters.
pub(super) fn text_prefix(text: &str, budget: usize) -> Result<(String, usize), ErrorData> {
    let mut used = 0usize;
    let mut kept = String::new();
    for character in text.chars() {
        let next = used
            .checked_add(json_char_len(character))
            .ok_or_else(|| room_error("measuring a text prefix"))?;
        if !kept.is_empty() && next > budget {
            break;
        }
        used = next;
        kept.push(character);
    }
    let chars = kept.chars().count();
    Ok((kept, chars))
}
