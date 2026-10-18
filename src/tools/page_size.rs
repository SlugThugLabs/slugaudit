//! The effective page size for one `query` or `leads` call.
//!
//! A page is cut at the request's `page_bytes` when given, else at
//! `ResourceLimits::default_page_bytes`, and never above the hard memory
//! ceiling `max_query_response_bytes`. A request (or configured default) that
//! asks for more than the ceiling is clamped, and the response says so: the
//! caller must be able to see that a limit acted on it, and to what value.

use crate::model::ResourceLimits;
use rmcp::ErrorData;

/// The page size a call resolved to, and whether the ceiling changed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PageSize {
    /// Effective page size in serialized JSON bytes.
    pub(super) bytes: usize,
    /// The larger size that was asked for (by the request, or by the
    /// configured default) before the ceiling clamped it; `None` when nothing
    /// was clamped.
    pub(super) clamped_from: Option<usize>,
}

/// Resolves the effective page size.
///
/// # Errors
///
/// Returns `invalid_params` when `requested` is `0`: a zero-byte page is not
/// a page, and treating it as something else would be a silent substitution.
pub(super) fn resolve(
    requested: Option<usize>,
    limits: &ResourceLimits,
) -> Result<PageSize, ErrorData> {
    if requested == Some(0) {
        return Err(ErrorData::invalid_params(
            "page_bytes must be at least 1; omit it for the default page size",
            None,
        ));
    }
    let wanted = requested.unwrap_or(limits.default_page_bytes);
    let ceiling = limits.max_query_response_bytes;
    Ok(PageSize {
        bytes: wanted.min(ceiling),
        clamped_from: (wanted > ceiling).then_some(wanted),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_omitted_page_size_resolves_to_the_default() {
        let limits = ResourceLimits::default();

        let size = resolve(None, &limits).expect("default resolves");

        assert_eq!(size.bytes, limits.default_page_bytes);
        assert_eq!(size.clamped_from, None);
    }

    #[test]
    fn a_page_size_above_the_ceiling_is_clamped_and_reports_what_was_asked() {
        let limits = ResourceLimits {
            max_query_response_bytes: 1_000,
            ..ResourceLimits::default()
        };

        let size = resolve(Some(5_000), &limits).expect("clamps");

        assert_eq!((size.bytes, size.clamped_from), (1_000, Some(5_000)));
    }

    #[test]
    fn a_configured_default_above_the_ceiling_is_clamped_and_reported() {
        let limits = ResourceLimits {
            max_query_response_bytes: 100,
            default_page_bytes: 400,
            ..ResourceLimits::default()
        };

        let size = resolve(None, &limits).expect("clamps");

        assert_eq!((size.bytes, size.clamped_from), (100, Some(400)));
    }

    #[test]
    fn a_zero_page_size_is_refused_not_substituted() {
        let error = resolve(Some(0), &ResourceLimits::default()).expect_err("zero is invalid");

        assert!(error.message.contains("page_bytes"), "{}", error.message);
    }
}
