//! The enum variants a source declares and the production sites that
//! construct them, read out of the source text.

use crate::scan::code_before_line_comment;

/// The variant identifiers declared by `pub enum <name>` — parsed from the
/// declaration itself, never a hand-kept copy. `None` when the declaration is
/// absent, which the gate treats as "cannot compute a verdict" rather than as
/// "no variants".
pub(super) fn enum_variants(source: &str, name: &str) -> Option<Vec<String>> {
    let header = format!("pub enum {name}");
    let after_header = source.split_once(&header)?.1;
    let body = after_header.split_once('{')?.1;
    let mut variants = Vec::new();
    // Bracket nesting inside a variant payload: only a line that begins at
    // depth 0 declares a variant, and only a `}` at depth 0 ends the enum.
    // The stack holds one entry per open bracket, so its length is the depth.
    let mut open_brackets: Vec<char> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let code = code_before_line_comment(trimmed);
        let starts_nested = !open_brackets.is_empty();
        for character in code.chars() {
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
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("#!")
        {
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
/// hand side, or a block tail — and never a match arm, a pattern, or a
/// comparison.
///
/// The distinction is load-bearing: the SQL mirror (`kind_text`,
/// `as_sql_text`) enumerates every variant as a match arm, so counting any
/// occurrence would make this gate pass for a variant with no producer.
pub(super) fn producer_variants(source: &str, name: &str) -> Vec<String> {
    let needle = format!("{name}::");
    let mut produced = Vec::new();
    // Walk the occurrences from the last to the first: each split yields the
    // whole text before the qualifier and the text after it, so no offset is
    // ever added or sliced. The text after the qualifier stops at the next
    // occurrence, which cannot change the decision: only what directly
    // follows the variant name is read.
    let mut remaining = source;
    while let Some((before, after_qualifier)) = remaining.rsplit_once(&needle) {
        let is_identifier_character =
            |character: char| character.is_ascii_alphanumeric() || character == '_';
        let variant: String = after_qualifier
            .chars()
            .take_while(|character| is_identifier_character(*character))
            .collect();
        remaining = before;
        if variant.is_empty() {
            continue;
        }
        let after_variant = after_qualifier.trim_start_matches(is_identifier_character);
        if is_value_position(before, after_variant) {
            produced.push(variant);
        }
    }
    produced.reverse();
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
        Some(':' | '{') => true,
        // A lone `=` is an assignment or a struct field's `= value`; `==`,
        // `!=`, `<=`, `>=` are comparisons, not constructions.
        Some('=') => !matches!(reversed.next(), Some('=' | '!' | '<' | '>')),
        _ => false,
    }
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
