//! Token scanner state machine for source limit checking.
//!
//! One handler per lexer state; each consumes at least one byte so the
//! caller's loop always terminates. The cursor holds the bytes still to scan
//! and every step takes its bytes off the front by pattern, so no offset is
//! ever added or sliced. Newline accounting is shared
//! ([`ScanCursor::finish_line`]) so every state counts a line the same way.

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
/// Grouped because the values only ever travel together — each step reads and
/// writes all of them — so as separate `&mut` parameters one call with two
/// arguments transposed would compile and silently miscount. As one struct
/// the caller cannot get the order wrong.
pub(crate) struct ScanCursor<'a> {
    /// The bytes not yet scanned; where the next step resumes.
    rest: &'a [u8],
    /// Lexer state at the front of `rest`.
    state: State,
    /// Whether the current line has already seen a non-comment token.
    has_code: bool,
    /// One entry per open `/*` while inside a block comment.
    block_depth: Vec<()>,
    /// The `"` and hash run that close the open raw string while inside one.
    raw_closer: Vec<u8>,
    /// Whether the end-of-input event has been reported.
    closed: bool,
}

impl<'a> ScanCursor<'a> {
    /// A cursor at the start of `bytes`.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            rest: bytes,
            state: State::Normal,
            has_code: false,
            block_depth: Vec::new(),
            raw_closer: Vec::new(),
            closed: false,
        }
    }

    /// Scans one step. Returns `None` once the input is exhausted and the
    /// last line has been reported; otherwise `Some(counted)`, where
    /// `counted` is whether the step closed a physical line that held code.
    /// The final event closes the unterminated last line.
    pub(crate) fn next_event(&mut self) -> Option<bool> {
        let Some((&byte, tail)) = self.rest.split_first() else {
            if self.closed {
                return None;
            }
            self.closed = true;
            return Some(self.has_code);
        };
        Some(match self.state {
            State::LineComment => self.step_line_comment(byte, tail),
            State::BlockComment => self.step_block_comment(byte, tail),
            State::String => self.step_quoted(byte, tail, b'"'),
            State::Char => self.step_quoted(byte, tail, b'\''),
            State::RawString => self.step_raw_string(byte, tail),
            State::Normal => self.step_normal(byte, tail),
        })
    }

    /// Closes the current physical line: whether it held code, so the caller
    /// counts it.
    fn finish_line(&mut self) -> bool {
        std::mem::take(&mut self.has_code)
    }

    /// Consumes one byte, closing the line first when that byte is `\n`.
    fn advance_byte(&mut self, byte: u8, tail: &'a [u8]) -> bool {
        self.rest = tail;
        byte == b'\n' && self.finish_line()
    }

    fn step_line_comment(&mut self, byte: u8, tail: &'a [u8]) -> bool {
        self.rest = tail;
        if byte == b'\n' {
            self.state = State::Normal;
            return self.finish_line();
        }
        false
    }

    fn step_block_comment(&mut self, byte: u8, tail: &'a [u8]) -> bool {
        match (byte, tail) {
            (b'/', [b'*', after @ ..]) => {
                self.block_depth.push(());
                self.rest = after;
                false
            }
            (b'*', [b'/', after @ ..]) => {
                self.block_depth.pop();
                if self.block_depth.is_empty() {
                    self.state = State::Normal;
                }
                self.rest = after;
                false
            }
            _ => self.advance_byte(byte, tail),
        }
    }

    /// `"…"` strings and `'…'` chars: literal bytes are code, `\` escapes
    /// the next byte (an escaped newline, a string continuation, still ends
    /// its physical line), and `close` returns to normal scanning.
    fn step_quoted(&mut self, byte: u8, tail: &'a [u8], close: u8) -> bool {
        self.has_code = true;
        match (byte, tail) {
            (b'\\', [escaped, after @ ..]) => self.advance_byte(*escaped, after),
            _ if byte == close => {
                self.state = State::Normal;
                self.rest = tail;
                false
            }
            _ => self.advance_byte(byte, tail),
        }
    }

    fn step_raw_string(&mut self, byte: u8, tail: &'a [u8]) -> bool {
        self.has_code = true;
        // The closer starts with `"`, so it only matches at a `"` followed by
        // the open raw string's full hash run.
        if let Some(after) = self.rest.strip_prefix(self.raw_closer.as_slice()) {
            self.state = State::Normal;
            self.raw_closer.clear();
            self.rest = after;
            false
        } else {
            self.advance_byte(byte, tail)
        }
    }

    fn step_normal(&mut self, byte: u8, tail: &'a [u8]) -> bool {
        if let (b'/', [b'/', after @ ..]) = (byte, tail) {
            self.state = State::LineComment;
            self.rest = after;
            return false;
        }
        if let (b'/', [b'*', after @ ..]) = (byte, tail) {
            self.block_depth = vec![()];
            self.state = State::BlockComment;
            self.rest = after;
            return false;
        }
        if byte == b'"' {
            self.has_code = true;
            self.state = State::String;
            self.rest = tail;
            return false;
        }
        if byte == b'\'' && is_char_literal_start(self.rest) {
            self.has_code = true;
            self.state = State::Char;
            self.rest = tail;
            return false;
        }
        if byte == b'r'
            && let Some((closer, after)) = raw_string_start(tail)
        {
            self.raw_closer = closer;
            self.has_code = true;
            self.state = State::RawString;
            self.rest = after;
            return false;
        }
        if !byte.is_ascii_whitespace() {
            self.has_code = true;
        }
        self.advance_byte(byte, tail)
    }
}

/// `'(?:\\.|[^'\\\n])'` — opening quote, then either `\` followed by any
/// char or one char that isn't `'`, `\`, or newline, then closing `'`.
fn is_char_literal_start(bytes: &[u8]) -> bool {
    matches!(bytes, [b'\'', b'\\', _, b'\'', ..])
        || matches!(bytes, [b'\'', plain, b'\'', ..] if !matches!(plain, b'\'' | b'\\' | b'\n'))
}

/// `r(#+)?\"` — after the `r` (`after_r` is the text following it): an
/// optional hash run, then the opening quote. Returns the bytes that close
/// the raw string (a `"` and the same hash run) and the text after the
/// opening quote.
fn raw_string_start(after_r: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    let mut after_hashes = after_r;
    while let [b'#', next @ ..] = after_hashes {
        after_hashes = next;
    }
    let [b'"', after_quote @ ..] = after_hashes else {
        return None;
    };
    let hashes = after_r.iter().take_while(|byte| **byte == b'#').count();
    let mut closer = vec![b'"'];
    closer.extend(std::iter::repeat_n(b'#', hashes));
    Some((closer, after_quote))
}
