//! The effective page size for one `query` or `leads` call.
//!
//! A page is cut at the request's `page_bytes` when given, else at
//! `ResourceLimits::default_page_bytes`, and never above the hard memory
//! ceiling `max_query_response_bytes`. A request (or configured default) that
//! asks for more than the ceiling is clamped, and the response says so: the
//! effective `page_bytes` is reported, and a page the clamped budget cut says
//! `limited_by: ceiling`, so the caller can see that raising `page_bytes`
//! would return nothing more.

use super::page::PageLimit;
use crate::model::ResourceLimits;
use rmcp::ErrorData;

/// The page size a call resolved to, and whether the ceiling changed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PageSize {
    /// Effective page size in serialized JSON bytes.
    pub(super) bytes: usize,
    /// True when the size asked for (by the request, or by the configured
    /// default) was at or above the ceiling, so `bytes` is the ceiling and a
    /// larger request cannot enlarge the page.
    pub(super) at_ceiling: bool,
}

impl PageSize {
    /// What a page cut by this byte budget reports as its `limited_by`: the
    /// ceiling when the budget is the ceiling, the caller's budget otherwise.
    pub(super) fn byte_cut(self) -> PageLimit {
        if self.at_ceiling {
            PageLimit::Ceiling
        } else {
            PageLimit::PageBytes
        }
    }
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
        at_ceiling: wanted >= ceiling,
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
        assert!(!size.at_ceiling);
        assert_eq!(size.byte_cut(), PageLimit::PageBytes);
    }

    #[test]
    fn a_page_size_above_the_ceiling_is_clamped_and_a_cut_reports_the_ceiling() {
        let limits = ResourceLimits {
            max_query_response_bytes: 1_000,
            ..ResourceLimits::default()
        };

        let size = resolve(Some(5_000), &limits).expect("clamps");

        assert_eq!((size.bytes, size.byte_cut()), (1_000, PageLimit::Ceiling));
    }

    #[test]
    fn a_configured_default_above_the_ceiling_is_clamped_and_reported() {
        let limits = ResourceLimits {
            max_query_response_bytes: 100,
            default_page_bytes: 400,
            ..ResourceLimits::default()
        };

        let size = resolve(None, &limits).expect("clamps");

        assert_eq!((size.bytes, size.byte_cut()), (100, PageLimit::Ceiling));
    }

    #[test]
    fn a_page_size_exactly_at_the_ceiling_reports_the_ceiling_when_it_cuts() {
        let limits = ResourceLimits {
            max_query_response_bytes: 1_000,
            ..ResourceLimits::default()
        };

        let size = resolve(Some(1_000), &limits).expect("resolves");

        assert_eq!((size.bytes, size.byte_cut()), (1_000, PageLimit::Ceiling));
    }

    #[test]
    fn a_page_size_below_the_ceiling_reports_page_bytes_when_it_cuts() {
        let limits = ResourceLimits {
            max_query_response_bytes: 1_000,
            ..ResourceLimits::default()
        };

        let size = resolve(Some(999), &limits).expect("resolves");

        assert_eq!((size.bytes, size.byte_cut()), (999, PageLimit::PageBytes));
    }

    #[test]
    fn a_zero_page_size_is_refused_not_substituted() {
        let error = resolve(Some(0), &ResourceLimits::default()).expect_err("zero is invalid");

        assert!(error.message.contains("page_bytes"), "{}", error.message);
    }
}
