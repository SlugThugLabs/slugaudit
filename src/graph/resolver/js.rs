//! JS/TS-specific resolver helpers.
//!
//! Extracted from `src/graph/resolver.rs` so JavaScript/TypeScript's
//! `import ... from 'path'` quote-handling lives next to the language's
//! other concerns (the `js()` constructor in the generic config, which
//! is small enough to stay co-located with [`super::generic::GenericResolver`]).
//!
//! JS/TS is special-case for one reason: the language uses quoted
//! strings as module paths (`import x from 'y'`), unlike Python's
//! bareword imports. A trivial substring match for the quote would
//! catch too much, so we gate on the `from` keyword first — that's
//! the actual syntactic token that makes it a JS/TS-style import.

use super::path_helpers::extract_quoted_string;
use crate::graph::reference::ImportReference;

/// Extracts a JS/TS-style module path from `raw` import source.
///
/// Returns `Some(ImportReference)` when `raw` contains `from` followed
/// by a quoted string; `None` otherwise. The `from` gate intentionally
/// rejects Go's `import "fmt"` shape (which has no `from`): Go's
/// quoted imports would otherwise match the JS pattern.
pub(crate) fn extract_js_reference(raw: &str) -> Option<ImportReference> {
    let trimmed = raw.trim();
    if !trimmed.contains("from") {
        return None;
    }
    let text = extract_quoted_string(trimmed)?;
    Some(ImportReference { text })
}
