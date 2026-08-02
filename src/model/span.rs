use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// A line or column past `u32::MAX` is not a real source position;
/// saturating is a documented choice, not a silent truncation risk. Shared
/// by every caller that converts a parser's `usize` position into ours.
#[must_use]
pub fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start_byte: u64,
    pub end_byte: u64,
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpanError {
    #[error("span end byte precedes start byte")]
    ReversedBytes,
    #[error("span end position precedes start position")]
    ReversedPosition,
}

impl Span {
    /// Creates a zero-based, half-open byte span with zero-based positions.
    ///
    /// # Errors
    ///
    /// Returns an error if `end_byte` precedes `start_byte`, or if `end`
    /// precedes `start` as a line/column position.
    pub fn new(
        start_byte: u64,
        end_byte: u64,
        start: Position,
        end: Position,
    ) -> Result<Self, SpanError> {
        if end_byte < start_byte {
            return Err(SpanError::ReversedBytes);
        }
        if (end.line, end.column) < (start.line, start.column) {
            return Err(SpanError::ReversedPosition);
        }
        Ok(Self {
            start_byte,
            end_byte,
            start,
            end,
        })
    }

    #[must_use]
    pub fn byte_len(self) -> u64 {
        self.end_byte - self.start_byte
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_empty_eof_span() {
        let position = Position { line: 2, column: 4 };
        let span = Span::new(8, 8, position, position).expect("valid span");
        assert_eq!(span.byte_len(), 0);
    }

    #[test]
    fn rejects_reversed_ranges() {
        let position = Position { line: 0, column: 0 };
        assert_eq!(
            Span::new(2, 1, position, position),
            Err(SpanError::ReversedBytes)
        );
    }
}
