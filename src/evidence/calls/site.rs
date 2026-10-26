//! The one `Call` row builder every call producer feeds — the Rust walker,
//! its macro re-parse, and the embedded call queries — so resolution,
//! confidence and the payload shape cannot drift between languages.

use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, SpanAvailability};
use serde_json::json;

/// One call site, in the shape every producer hands to [`call_item`].
pub(super) struct CallSite {
    pub(super) language: &'static str,
    pub(super) origin: EvidenceOrigin,
    /// The callee as written, receiver included (`v.unwrap`, `obj.method`).
    pub(super) callee_text: String,
    /// The callee's last simple name (`unwrap`, `method`).
    pub(super) callee_name: String,
    pub(super) caller: Option<String>,
    pub(super) macro_call: bool,
    /// The callee is a `::` path (see [`is_qualified`]).
    pub(super) qualified: bool,
    pub(super) span: SpanAvailability,
}

/// Whether a callee's text is a `::`-qualified path. Only Rust writes one
/// today; the rule is shared so every language classifies the same way.
pub(super) fn is_qualified(callee_text: &str) -> bool {
    callee_text.contains("::")
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
        payload: json!({
            "language": site.language,
            "callee_text": site.callee_text,
            "callee_name": site.callee_name,
            "caller": site.caller,
            "resolution": resolution,
            "confidence": confidence,
        }),
    }
}
