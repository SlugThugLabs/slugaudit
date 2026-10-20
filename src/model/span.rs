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

/// A line or column past `u32::MAX` is not a real source position. The clamp
/// is unreachable for any file the profiles admit (`max_file_bytes` is at most
/// 1 GiB, so no line or column count reaches 2^32), and it is logged if it
/// ever happens. Shared by every caller that converts a parser's `usize`
/// position into ours.
#[must_use]
pub fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or_else(|error| {
        tracing::warn!(value, %error, "source position exceeds u32::MAX; clamped");
        u32::MAX
    })
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

/// Bytes per block of the character-count table in [`LineIndex`]. A column
/// query scans at most this many bytes, so it bounds the per-query cost
/// independent of line length; the table costs `8 / BLOCK_BYTES` of the
/// source size (about 3 percent).
const BLOCK_BYTES: usize = 256;

/// Per-file line/column lookup, built once in O(n) and answering each query
/// in O(log lines + `BLOCK_BYTES`) — independent of line length.
///
/// [`char_column`] walks back to the line start per call, so a minified
/// single-line file with one evidence item per token costs line-length
/// steps per item: quadratic in file size. This index replaces it for every
/// bulk caller. Results are identical to [`char_column`] and to counting
/// `\n` bytes before the offset, including for offsets past the end of the
/// source or inside a multi-byte character (floored to the boundary).
#[derive(Debug)]
pub struct LineIndex<'a> {
    source: &'a str,
    /// Byte offset of the start of every line; always begins with 0.
    line_starts: Vec<usize>,
    /// `block_chars[i]` is the number of characters before byte
    /// `i * BLOCK_BYTES`. Empty when the source is pure ASCII, where a
    /// character count equals a byte count.
    block_chars: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// Indexes `source` once.
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        let bytes = source.as_bytes();
        let mut line_starts = vec![0];
        line_starts.extend(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        let mut block_chars = Vec::new();
        if !source.is_ascii() {
            let mut total = 0usize;
            block_chars.push(0);
            for chunk in bytes.chunks(BLOCK_BYTES) {
                total += count_chars(chunk);
                block_chars.push(total);
            }
        }
        Self {
            source,
            line_starts,
            block_chars,
        }
    }

    /// Zero-based line of `byte_offset` (clamped to the source length).
    /// Used by `span_tests`; production callers use [`LineIndex::position`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn line(&self, byte_offset: usize) -> u32 {
        saturating_u32(self.line_index(byte_offset.min(self.source.len())))
    }

    /// Character column of `byte_offset`; equal to
    /// [`char_column`]`(source, byte_offset)`.
    #[must_use]
    pub fn column(&self, byte_offset: usize) -> u32 {
        self.position(byte_offset).column
    }

    /// Line and column of `byte_offset` in one lookup.
    #[must_use]
    pub fn position(&self, byte_offset: usize) -> Position {
        let offset = self.floor_to_boundary(byte_offset);
        let line = self.line_index(offset);
        let line_start = self.line_starts[line];
        Position {
            line: saturating_u32(line),
            column: saturating_u32(self.chars_before(offset) - self.chars_before(line_start)),
        }
    }

    fn floor_to_boundary(&self, byte_offset: usize) -> usize {
        let mut offset = byte_offset.min(self.source.len());
        // Offset 0 is always a char boundary, so this terminates.
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// Index of the line containing `offset`: the number of `\n` bytes
    /// strictly before it.
    fn line_index(&self, offset: usize) -> usize {
        // `line_starts[0] == 0 <= offset`, so the partition point is >= 1.
        self.line_starts.partition_point(|start| *start <= offset) - 1
    }

    /// Characters in `source[..offset]`; `offset` must be a char boundary.
    fn chars_before(&self, offset: usize) -> usize {
        if self.block_chars.is_empty() {
            return offset;
        }
        let block = offset / BLOCK_BYTES;
        let tail = &self.source.as_bytes()[block * BLOCK_BYTES..offset];
        self.block_chars[block] + count_chars(tail)
    }
}

/// Characters starting in `bytes`: every byte that is not a UTF-8
/// continuation byte.
fn count_chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| (**byte & 0xC0) != 0x80).count()
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

    /// Length of the span in bytes, or `None` when `end_byte` precedes
    /// `start_byte` (possible for a deserialized `Span`, which bypasses
    /// [`Span::new`]).
    #[must_use]
    pub fn byte_len(self) -> Option<u64> {
        self.end_byte.checked_sub(self.start_byte)
    }
}

#[cfg(test)]
#[path = "span_tests.rs"]
mod tests;
