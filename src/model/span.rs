use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Zero-based line and character column position within a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Zero-based line index.
    pub line: u32,
    /// A **character** offset from the start of the line (counting Unicode
    /// scalar values, i.e. `str::chars()`), not a byte offset. Tree-sitter's
    /// own `Point::column` is a byte offset, so every caller that derives a
    /// `Position` from a Tree-sitter node must convert via [`char_column`]
    /// rather than copying `Point::column` directly — otherwise any source
    /// line containing a multi-byte UTF-8 character reports a column too
    /// large for everything after it on that line.
    pub column: u32,
}

/// A line or column past `u32::MAX` is not a real source position;
/// saturating is a documented choice, not a silent truncation risk. Shared
/// by every caller that converts a parser's `usize` position into ours.
#[must_use]
pub fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Converts a Tree-sitter byte offset into the character offset from the
/// start of its line, for use as [`Position::column`]. Tree-sitter's
/// `Point::column` counts bytes since the last `\n` (or the start of
/// `source`), not characters — copying it directly produces a column too
/// large for any position after a multi-byte UTF-8 character on the same
/// line.
///
/// A Tree-sitter node boundary lands on a char boundary for a well-formed
/// parse, but this does not require one: an offset that lands mid-codepoint,
/// or past the end of the source, is floored to the nearest boundary rather
/// than panicking. A `structure` call that dies on real project content is
/// worse than a column measured one character early.
#[must_use]
pub fn char_column(source: &str, byte_offset: usize) -> u32 {
    let mut offset = byte_offset.min(source.len());
    // Offset 0 is always a char boundary, so this terminates.
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let line_start = source[..offset].rfind('\n').map_or(0, |idx| idx + 1);
    saturating_u32(source[line_start..offset].chars().count())
}

/// Half-open range of source text spanning byte offsets and line/column positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// Zero-based start byte offset (inclusive).
    pub start_byte: u64,
    /// Zero-based end byte offset (exclusive).
    pub end_byte: u64,
    /// Start position in line and column coordinates.
    pub start: Position,
    /// End position in line and column coordinates.
    pub end: Position,
}

/// Validation errors when constructing a [`Span`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpanError {
    /// End byte offset is strictly smaller than start byte offset.
    #[error("span end byte precedes start byte")]
    ReversedBytes,
    /// End line/column position precedes start position.
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

    /// Length of the span in bytes.
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

    #[test]
    fn a_mid_codepoint_offset_floors_to_the_character_boundary() {
        // `é` occupies bytes 0..2, so byte 1 is inside it: the column must be
        // measured from the boundary before it rather than panicking.
        let source = "é=1";
        assert_eq!(char_column(source, 1), 0);
        assert_eq!(char_column(source, 2), 1);
    }

    #[test]
    fn an_offset_past_the_end_measures_the_whole_line() {
        let source = "a\nbc";
        assert_eq!(char_column(source, 99), 2);
    }
}
