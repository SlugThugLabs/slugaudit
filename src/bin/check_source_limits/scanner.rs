//! Token scanner state machine for source limit checking.
//!
//! One handler per lexer state; each consumes at least one byte so the
//! caller's `while cursor.index < len` loop always terminates. Newline
//! accounting is shared ([`ScanCursor::finish_line`]) so every state counts
//! a line the same way.

#[derive(Default)]
pub(crate) enum State {
    #[default]
    Normal,
    LineComment,
    BlockComment,
    String,
    Char,
    RawString,
}

/// The scanner's mutable cursor: every value a step advances or accumulates.
///
/// Grouped because the six only ever travel together — each step reads and
/// writes all of them — so as six separate `&mut` parameters one call with
/// two arguments transposed would compile and silently miscount. As one
/// struct the caller cannot get the order wrong.
#[derive(Default)]
pub(crate) struct ScanCursor {
    /// Byte offset into the source; where the next step resumes.
    pub(crate) index: usize,
    /// Lexer state at `index`.
    pub(crate) state: State,
    /// Whether the current line has already seen a non-comment token.
    pub(crate) has_code: bool,
    /// Completed code lines so far.
    pub(crate) count: usize,
    /// `/*` nesting depth while inside a block comment.
    pub(crate) block_depth: usize,
    /// Hash count of the open raw string while inside one.
    pub(crate) raw_hashes: usize,
}

pub(crate) fn scan_step(bytes: &[u8], source: &str, cursor: &mut ScanCursor) {
    match cursor.state {
        State::LineComment => cursor.step_line_comment(bytes),
        State::BlockComment => cursor.step_block_comment(bytes),
        State::String => cursor.step_quoted(bytes, b'"'),
        State::Char => cursor.step_quoted(bytes, b'\''),
        State::RawString => cursor.step_raw_string(bytes),
        State::Normal => cursor.step_normal(bytes, source),
    }
}

impl ScanCursor {
    /// Closes the current physical line: counts it when it held code.
    fn finish_line(&mut self) {
        if self.has_code {
            self.count += 1;
            self.has_code = false;
        }
    }

    /// Consumes one byte, closing the line first when that byte is `\n`.
    fn advance_byte(&mut self, byte: u8) {
        if byte == b'\n' {
            self.finish_line();
        }
        self.index += 1;
    }

    /// Whether the two bytes at `index` are exactly `pair`.
    fn at_pair(&self, bytes: &[u8], pair: [u8; 2]) -> bool {
        bytes.get(self.index..self.index + 2) == Some(&pair[..])
    }

    /// Moves to `state`, consuming `width` opener bytes.
    fn enter(&mut self, state: State, width: usize) {
        self.state = state;
        self.index += width;
    }

    fn step_line_comment(&mut self, bytes: &[u8]) {
        if bytes[self.index] == b'\n' {
            self.finish_line();
            self.state = State::Normal;
        }
        self.index += 1;
    }

    fn step_block_comment(&mut self, bytes: &[u8]) {
        if self.at_pair(bytes, *b"/*") {
            self.block_depth += 1;
            self.index += 2;
        } else if self.at_pair(bytes, *b"*/") {
            self.block_depth -= 1;
            if self.block_depth == 0 {
                self.state = State::Normal;
            }
            self.index += 2;
        } else {
            self.advance_byte(bytes[self.index]);
        }
    }

    /// `"…"` strings and `'…'` chars: literal bytes are code, `\` escapes
    /// the next byte, and `close` returns to normal scanning.
    fn step_quoted(&mut self, bytes: &[u8], close: u8) {
        self.has_code = true;
        let byte = bytes[self.index];
        if byte == b'\\' && self.index + 1 < bytes.len() {
            self.index += 2;
        } else if byte == close {
            self.enter(State::Normal, 1);
        } else {
            self.advance_byte(byte);
        }
    }

    fn step_raw_string(&mut self, bytes: &[u8]) {
        self.has_code = true;
        let byte = bytes[self.index];
        if byte == b'"' && self.closes_raw_string(bytes) {
            self.enter(State::Normal, self.raw_hashes + 1);
        } else {
            self.advance_byte(byte);
        }
    }

    /// Whether the `"` at `index` is followed by the open raw string's
    /// full hash run.
    fn closes_raw_string(&self, bytes: &[u8]) -> bool {
        self.index + self.raw_hashes < bytes.len()
            && (1..=self.raw_hashes).all(|i| bytes[self.index + i] == b'#')
    }

    fn step_normal(&mut self, bytes: &[u8], source: &str) {
        let byte = bytes[self.index];
        if self.at_pair(bytes, *b"//") {
            self.enter(State::LineComment, 2);
        } else if self.at_pair(bytes, *b"/*") {
            self.block_depth = 1;
            self.enter(State::BlockComment, 2);
        } else if byte == b'"' {
            self.has_code = true;
            self.enter(State::String, 1);
        } else if byte == b'\'' && is_char_literal_start(&source[self.index..]) {
            self.has_code = true;
            self.enter(State::Char, 1);
        } else if let Some(hashes) = (byte == b'r')
            .then(|| is_raw_string_start(&source[self.index..]))
            .flatten()
        {
            // Sliced only after an ASCII `r`, which is always a char boundary.
            self.raw_hashes = hashes;
            self.has_code = true;
            self.enter(State::RawString, hashes + 2);
        } else {
            if !byte.is_ascii_whitespace() {
                self.has_code = true;
            }
            self.advance_byte(byte);
        }
    }
}

/// `'(?:\\.|[^'\\\n])'` — opening quote, then either `\` followed by any
/// char or one char that isn't `'`, `\`, or newline, then closing `'`.
fn is_char_literal_start(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'\'' {
        return false;
    }
    if bytes[1] == b'\\' {
        return bytes.len() >= 4 && bytes[3] == b'\'';
    }
    if matches!(bytes[1], b'\'' | b'\\' | b'\n') {
        return false;
    }
    bytes[2] == b'\''
}

/// `r(#+)?\"` — `r`, optional hash run, then opening quote. Returns the
/// hash count (0 for `r"…"`, 5 for `r#####"…"`, etc.).
fn is_raw_string_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'r') {
        return None;
    }
    let n_hashes = bytes[1..].iter().take_while(|&&b| b == b'#').count();
    (bytes.get(1 + n_hashes) == Some(&b'"')).then_some(n_hashes)
}
