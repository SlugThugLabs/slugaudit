//! Result ids and the cursor codec: `v1.<nonce>.<result-uuid>.<ord>`.
//!
//! The position lives in the token, so a repeated call returns the same page.
//! Decoding is strict: anything but the canonical form is `Unknown`.

use super::error::ResultStoreError;
use std::fmt;
use uuid::Uuid;

/// Longest token the decoder looks at (the canonical form is at most 96).
const MAX_TOKEN_LEN: usize = 128;

/// The random identity of one stored result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResultId(pub(super) Uuid);

impl fmt::Display for ResultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

/// A decoded cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// The process that issued it.
    pub nonce: Uuid,
    /// The result it reads.
    pub result: ResultId,
    /// The first ordinal of the page.
    pub ord: u64,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "v1.{}.{}.{}",
            self.nonce.hyphenated(),
            self.result,
            self.ord
        )
    }
}

impl Cursor {
    /// Parses a token.
    ///
    /// # Errors
    ///
    /// `Unknown` for every malformed token. Not logged here; the store counts it.
    pub fn decode(token: &str) -> Result<Self, ResultStoreError> {
        Self::parse(token).ok_or(ResultStoreError::Unknown)
    }

    pub(super) fn parse(token: &str) -> Option<Self> {
        if token.len() > MAX_TOKEN_LEN || !token.is_ascii() {
            return None;
        }
        let mut parts = token.split('.');
        let (version, nonce, result, ord) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || version != "v1" {
            return None;
        }
        Some(Self {
            nonce: canonical_uuid(nonce)?,
            result: ResultId(canonical_uuid(result)?),
            ord: canonical_ord(ord)?,
        })
    }
}

/// Only the lowercase hyphenated form is accepted.
fn canonical_uuid(text: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(text).ok()?;
    (id.hyphenated().to_string() == text).then_some(id)
}

/// Plain decimal: digits only, no sign, no leading zero except "0".
fn canonical_ord(text: &str) -> Option<u64> {
    let plain = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if plain { text.parse().ok() } else { None }
}
