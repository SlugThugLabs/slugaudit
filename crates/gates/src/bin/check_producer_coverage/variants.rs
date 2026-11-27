//! The enum variants a source declares and the production sites that
//! construct them, read out of the source text.

use crate::scan::strip_non_code;

/// The variant identifiers declared by `pub enum <name>` — parsed from the
/// declaration itself, never a hand-kept copy. `None` when the declaration is
/// absent, which the gate treats as "cannot compute a verdict" rather than as
/// "no variants".
pub(super) fn enum_variants(source: &str, name: &str) -> Option<Vec<String>> {
    let clean = strip_non_code(source);
    let header = format!("pub enum {name}");
    let after_header = clean.split_once(&header)?.1;
    let body = after_header.split_once('{')?.1;
    let mut variants = Vec::new();
    // Bracket nesting inside a variant payload: only a line that begins at
    // depth 0 declares a variant, and only a `}` at depth 0 ends the enum.
    // The stack holds one entry per open bracket, so its length is the depth.
    let mut open_brackets: Vec<char> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let starts_nested = !open_brackets.is_empty();
        for character in trimmed.chars() {
            match character {
                '{' | '(' | '[' => open_brackets.push(character),
                // A closer at depth 0 is the enum's own closing brace.
                '}' | ')' | ']' if open_brackets.pop().is_none() => return Some(variants),
                _ => {}
            }
        }
        if starts_nested {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("#[") || trimmed.starts_with("#!") {
            continue;
        }
        // A variant is the leading identifier of its line; a payload
        // (`LoadFailed { … }`) or discriminant never changes which variant
        // the line declares.
        let head = match trimmed.split_once(['{', '(', '=', ',']) {
            Some((head, _)) => head,
            None => trimmed,
        }
        .trim();
        if is_identifier(head) {
            variants.push(head.to_owned());
        }
    }
    Some(variants)
}

/// The variants of `name` constructed in `source` at a *value position* — a
/// struct-literal field (`kind: EvidenceKind::X`), a `let`/assignment right
/// hand side, a block tail, or a function-call argument
/// (`emit(EvidenceKind::X)`, `emit(ctx, EvidenceKind::X)`) — and never a match
/// arm, a `matches!` / destructuring pattern, a comment, a string literal, or
/// a comparison.
///
/// The distinction is load-bearing: the SQL mirror (`kind_text`,
/// `as_sql_text`) enumerates every variant as a match arm, so counting any
/// occurrence would make this gate pass for a variant with no producer.
pub(super) fn producer_variants(source: &str, name: &str) -> Vec<String> {
    let clean = strip_non_code(source);
    let needle = format!("{name}::");
    let is_ident = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let mut produced = Vec::new();
    for (byte_pos, _) in clean.match_indices(&needle) {
        let Some(before) = clean.get(..byte_pos) else {
            continue;
        };
        if before.chars().next_back().is_some_and(is_ident) {
            continue;
        }
        let Some(after_pos) = byte_pos.checked_add(needle.len()) else {
            continue;
        };
        let Some(after_qualifier) = clean.get(after_pos..) else {
            continue;
        };
        let variant: String = after_qualifier
            .chars()
            .take_while(|character| is_ident(*character))
            .collect();
        if variant.is_empty() {
            continue;
        }
        let after_variant = after_qualifier.trim_start_matches(is_ident);
        if is_value_position(before, after_variant) {
            produced.push(variant);
        }
    }
    produced
}

/// Whether the variant, written between `before` and `after_variant`, is
/// constructed rather than matched or compared.
fn is_value_position(before: &str, after_variant: &str) -> bool {
    if after_variant.trim_start().starts_with("=>") {
        return false;
    }
    let before = without_path_prefix(before.trim_end());
    let mut reversed = before.chars().rev();
    match reversed.next() {
        Some(':' | '{') => !closes_into_match_arm(after_variant, '{', '}'),
        // A lone `=` is an assignment or a struct field's `= value`; `==`,
        // `!=`, `<=`, `>=` are comparisons, not constructions.
        Some('=') => !matches!(reversed.next(), Some('=' | '!' | '<' | '>')),
        Some('(' | ',') => is_argument_value_position(before, after_variant),
        _ => false,
    }
}

/// Whether `Enum::Variant` preceded by `(` or `,` is a call/tuple value
/// construction rather than a `matches!(...)` arm, a `match` tuple/constructor
/// pattern (`Some(Enum::Variant) =>`), or a `let` destructuring pattern
/// (`if let Some(Enum::Variant) = ...`).
fn is_argument_value_position(before: &str, after_variant: &str) -> bool {
    let unclosed_parens = unclosed_open_paren_offsets(before);
    if unclosed_parens.is_empty() {
        return false;
    }
    // Reject `matches!(expr, Enum::Variant)` and nested patterns inside `matches!(...)`.
    for &open_offset in &unclosed_parens {
        if let Some(prefix) = before.get(..open_offset)
            && prefix.trim_end().ends_with("matches!")
        {
            return false;
        }
    }
    // Reject `Some(Enum::Variant) =>` and `(Enum::Variant, _) =>` match arms.
    if closes_into_match_arm(after_variant, '(', ')') {
        return false;
    }
    // Reject `if let Some(Enum::Variant) = x` and `let (Enum::Variant, _) = x`.
    if let Some(&outer_open) = unclosed_parens.first()
        && let Some(prefix) = before.get(..outer_open)
        && closes_into_let_binding(prefix, after_variant)
    {
        return false;
    }
    true
}

/// Byte offsets of every unclosed `(` in `before`, from outermost to innermost.
fn unclosed_open_paren_offsets(before: &str) -> Vec<usize> {
    let mut stack = Vec::new();
    for (offset, ch) in before.char_indices() {
        match ch {
            '(' => stack.push(offset),
            ')' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack
}

/// Whether any enclosing `close` delimiter in `after_variant` is followed by
/// `=>` (making this group a `match` arm pattern).
fn closes_into_match_arm(after_variant: &str, open: char, close: char) -> bool {
    let mut depth: usize = 1;
    for (offset, ch) in after_variant.char_indices() {
        if ch == open {
            let Some(next_depth) = depth.checked_add(1) else {
                return false;
            };
            depth = next_depth;
        } else if ch == close {
            let Some(next_depth) = depth.checked_sub(1) else {
                return false;
            };
            depth = next_depth;
            if depth == 0 {
                let Some(after_close_pos) = offset.checked_add(ch.len_utf8()) else {
                    return false;
                };
                return after_variant
                    .get(after_close_pos..)
                    .is_some_and(|tail| tail.trim_start().starts_with("=>"));
            }
        }
    }
    false
}

/// Whether `prefix` starts a `let` / `if let` / `while let` pattern before `(`
/// and `after_variant` closes its outer `)` into a lone `=` binding.
fn closes_into_let_binding(prefix: &str, after_variant: &str) -> bool {
    let statement_head = match prefix.rfind([';', '{', '}']) {
        Some(pos) => match pos.checked_add(1).and_then(|start| prefix.get(start..)) {
            Some(tail) => tail,
            None => prefix,
        },
        None => prefix,
    };
    // If `=` already appeared before `(`, we are on the value RHS (`let x = f(Thing::Alpha)`).
    if statement_head.contains('=') {
        return false;
    }
    let has_let_keyword = statement_head
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| word == "let");
    if !has_let_keyword {
        return false;
    }
    let mut depth: usize = 1;
    for (offset, ch) in after_variant.char_indices() {
        match ch {
            '(' => {
                let Some(next_depth) = depth.checked_add(1) else {
                    return false;
                };
                depth = next_depth;
            }
            ')' => {
                let Some(next_depth) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next_depth;
                if depth == 0 {
                    let Some(after_close_pos) = offset.checked_add(ch.len_utf8()) else {
                        return false;
                    };
                    let Some(tail) = after_variant.get(after_close_pos..) else {
                        return false;
                    };
                    let trimmed = tail.trim_start();
                    return trimmed.starts_with('=')
                        && !trimmed.starts_with("==")
                        && !trimmed.starts_with("=>");
                }
            }
            _ => {}
        }
    }
    false
}

/// `before` without the module path that qualifies the variant:
/// `x == model::EvidenceKind::Call` is a comparison, so the `==` before the
/// path decides, not the `::` that ends it. A lone `:` (a struct field) is
/// left alone.
fn without_path_prefix(mut before: &str) -> &str {
    while let Some(path) = before.strip_suffix("::") {
        before = path
            .trim_end_matches(|character: char| {
                character.is_ascii_alphanumeric() || character == '_'
            })
            .trim_end();
    }
    before
}

fn is_identifier(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}
