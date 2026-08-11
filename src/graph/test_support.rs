//! Shared fixture helper for `graph` tests. `paths` builds the
//! `known_paths` argument most resolution tests pass; it was previously
//! copied in resolve_tests.rs, mod_tests.rs, and resolve_rust_tests.rs.
use std::collections::HashSet;

/// Builds the `HashSet<&str>` of known project paths for `resolve_*` calls.
pub(crate) fn paths<'a>(list: &[&'a str]) -> HashSet<&'a str> {
    list.iter().copied().collect()
}
