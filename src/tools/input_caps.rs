//! The byte caps every tool applies to the same kind of caller-supplied input,
//! defined once so a `path` or a `cursor` is bounded alike on every tool and
//! each refusal message is formatted from the cap it enforces.

/// Longest `path` (or path-shaped field, such as `start_after_file`) a tool
/// accepts, checked before the value is used.
pub(crate) const MAX_PATH_BYTES: usize = 32 * 1024;

/// Longest `cursor` a tool accepts. A cursor the server issues is at most 96
/// bytes (`store::result_store` cursor codec), so anything over this cap can
/// never name a stored result and is refused before it is decoded.
pub(crate) const MAX_CURSOR_BYTES: usize = 1024;
