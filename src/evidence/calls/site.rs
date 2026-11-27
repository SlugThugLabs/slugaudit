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

/// Whether a callee's text contains a `::` path separator outside string
/// literals (`"a::b".trim()` is an unqualified method call on a string literal,
/// whereas `crate::util::now` is a qualified Rust path).
pub(super) fn is_qualified(callee_text: &str) -> bool {
    let mut rest = callee_text.as_bytes();
    let mut in_quote: Option<u8> = None;
    while let Some((&byte, tail)) = rest.split_first() {
        if let Some(quote) = in_quote {
            if byte == b'\\' {
                rest = match tail.split_first() {
                    Some((_, after_escaped)) => after_escaped,
                    None => &[],
                };
                continue;
            }
            if byte == quote {
                in_quote = None;
            }
            rest = tail;
            continue;
        }
        if byte == b'"' {
            in_quote = Some(b'"');
            rest = tail;
            continue;
        }
        if byte == b':' && tail.first() == Some(&b':') {
            return true;
        }
        rest = tail;
    }
    false
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
