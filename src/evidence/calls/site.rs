//! The one `Call` row builder every call producer feeds — the Rust walker,
//! its macro re-parse, and the embedded call queries — so resolution,
//! confidence and the payload shape cannot drift between languages.

use crate::model::{
    CallPayload, EvidenceItem, EvidenceKind, EvidenceOrigin, EvidencePayload, SpanAvailability,
};

/// One call site, in the shape every producer hands to [`call_item`].
pub(super) struct CallSite {
    pub(super) language: &'static str,
    pub(super) origin: EvidenceOrigin,
    /// The callee as written, receiver included (`v.expect`, `obj.method`).
    pub(super) callee_text: String,
    /// The callee's last simple name (`expect`, `method`).
    pub(super) callee_name: String,
    pub(super) caller: Option<String>,
    pub(super) macro_call: bool,
    /// The callee is a `::` path (see [`is_qualified`]).
    pub(super) qualified: bool,
    pub(super) span: SpanAvailability,
}

/// Whether a callee's text contains a `::` path separator outside string and
/// character literals (`"a::b".trim()`, `r#"a::b"#.trim()`, and `('"').trim()`
/// are unqualified, whereas `crate::util::now` is a qualified Rust path).
pub(super) fn is_qualified(callee_text: &str) -> bool {
    let mut rest = callee_text;
    let mut prev_ident = false;
    while !rest.is_empty() {
        if let Some(after) = skip_raw_string(rest, prev_ident) {
            rest = after;
            prev_ident = false;
            continue;
        }
        if let Some(after) = rest.strip_prefix('"') {
            rest = skip_quoted_string(after);
            prev_ident = false;
            continue;
        }
        if rest.starts_with('\'')
            && let Some(after) = skip_char_literal(rest)
        {
            rest = after;
            prev_ident = false;
            continue;
        }
        if rest.starts_with("::") {
            return true;
        }
        let mut chars = rest.chars();
        if let Some(ch) = chars.next() {
            prev_ident = ch.is_ascii_alphanumeric() || ch == '_';
            rest = chars.as_str();
        }
    }
    false
}

fn skip_quoted_string(after_open: &str) -> &str {
    let mut chars = after_open.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if chars.next().is_none() {
                break;
            }
        } else if ch == '"' {
            return chars.as_str();
        }
    }
    ""
}

fn skip_raw_string(rest: &str, prev_ident: bool) -> Option<&str> {
    if prev_ident {
        return None;
    }
    let after_r = rest
        .strip_prefix("br")
        .or_else(|| rest.strip_prefix("cr"))
        .or_else(|| rest.strip_prefix('r'))?;
    let hashes = after_r.bytes().take_while(|&b| b == b'#').count();
    let after_hashes = after_r.get(hashes..)?;
    let mut inside = after_hashes.strip_prefix('"')?;
    while let Some((_, after_quote)) = inside.split_once('"') {
        let trailing_hashes = after_quote.bytes().take_while(|&b| b == b'#').count();
        if trailing_hashes >= hashes {
            return after_quote.get(hashes..);
        }
        inside = after_quote;
    }
    Some("")
}

fn skip_char_literal(rest: &str) -> Option<&str> {
    let after_open = rest.strip_prefix('\'')?;
    if let Some(after_slash) = after_open.strip_prefix('\\') {
        let mut chars = after_slash.chars();
        let first = chars.next()?;
        if first == 'x' {
            let d1 = chars.next()?;
            let d2 = chars.next()?;
            return (d1.is_ascii_hexdigit()
                && d2.is_ascii_hexdigit()
                && chars.next() == Some('\''))
            .then_some(chars.as_str());
        }
        if first == 'u'
            && let Some(in_brace) = chars.as_str().strip_prefix('{')
        {
            let mut hex_chars = in_brace.chars();
            let mut has_hex = false;
            for ch in hex_chars.by_ref() {
                if ch == '}' {
                    return if has_hex {
                        hex_chars.as_str().strip_prefix('\'')
                    } else {
                        None
                    };
                }
                if ch.is_ascii_hexdigit() {
                    has_hex = true;
                } else if ch != '_' {
                    return None;
                }
            }
            return None;
        }
        return (chars.next() == Some('\'')).then_some(chars.as_str());
    }
    let mut chars = after_open.chars();
    let ch = chars.next()?;
    (ch != '\'' && ch != '\n' && ch != '\r' && chars.next() == Some('\'')).then_some(chars.as_str())
}

/// The one `Call` row builder: resolution and confidence from the site's
/// shape — `macro`/high, `qualified-syntactic`/high, else
/// `unresolved-syntactic`/medium — for every producer and language.
pub(super) fn call_item(key: String, site: CallSite) -> EvidenceItem {
    let (resolution, confidence) = if site.macro_call {
        ("macro", "high")
    } else if site.qualified {
        ("qualified-syntactic", "high")
    } else {
        ("unresolved-syntactic", "medium")
    };
    EvidenceItem {
        key,
        kind: EvidenceKind::Call,
        origin: site.origin,
        span: site.span,
        payload: EvidencePayload::Call(CallPayload {
            language: site.language.to_owned(),
            callee_text: site.callee_text,
            callee_name: site.callee_name,
            caller: site.caller,
            resolution: resolution.to_owned(),
            confidence: confidence.to_owned(),
        }),
    }
}
