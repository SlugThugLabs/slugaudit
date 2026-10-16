//! Lexical scanning for SQLite statements.
//!
//! Locates the first statement-terminating semicolon outside string literals
//! (`'...'`, `"..."`, `` `...` ``, `[...]`) and comments (`--...`, `/*...*/`).

/// Scans `sql` for a top-level statement-terminating semicolon outside string
/// literals, quoted identifiers, and line/block comments.
///
/// Returns the byte offset of the semicolon if one exists, or `None` if the
/// string is a single statement or contains no top-level separator.
pub(crate) fn first_statement_separator(sql: &str) -> Option<usize> {
    let mut scan = Scan::new(sql.as_bytes());
    while scan.more() {
        // Order is load-bearing: an active quote/bracket state must be
        // consumed *before* comment detection, or a `--` or `/*` inside a
        // string literal would be mistaken for a comment and hide the real
        // separator (the bug an earlier version of this scanner had).
        if scan.skip_quoted() {
            continue;
        }
        if scan.skip_comment() {
            continue;
        }
        if let Some(offset) = scan.enter_delimiter_or_separator() {
            return Some(offset);
        }
    }
    None
}

/// Byte cursor carrying which quoting context we are currently inside. At
/// most one flag is set at a time, because a delimiter is only recorded when
/// none of the states is active (the caller checks `skip_quoted` first).
struct Scan<'a> {
    bytes: &'a [u8],
    i: usize,
    in_single: bool,
    in_double: bool,
    in_backtick: bool,
    in_bracket: bool,
}

impl<'a> Scan<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            i: 0,
            in_single: false,
            in_double: false,
            in_backtick: false,
            in_bracket: false,
        }
    }

    fn more(&self) -> bool {
        self.i < self.bytes.len()
    }

    /// The byte at the cursor; callers guard with [`Scan::more`].
    fn at(&self) -> u8 {
        self.bytes[self.i]
    }

    /// The byte one past the cursor, if any.
    fn next(&self) -> Option<u8> {
        self.bytes.get(self.i + 1).copied()
    }

    /// Advances one byte, or two for a doubled quote, while inside a quoting
    /// context. Returns `true` if a context consumed the byte (so the caller
    /// restarts the loop) and `false` if no quoting context is active.
    fn skip_quoted(&mut self) -> bool {
        // Inside `'...'` or `"..."`: a doubled delimiter is an escaped
        // literal delimiter (stay inside, skip the pair); a single one
        // closes the run.
        if self.in_single {
            if self.at() == b'\'' {
                if self.next() == Some(b'\'') {
                    self.i += 1;
                } else {
                    self.in_single = false;
                }
            }
            self.i += 1;
            return true;
        }
        if self.in_double {
            if self.at() == b'"' {
                if self.next() == Some(b'"') {
                    self.i += 1;
                } else {
                    self.in_double = false;
                }
            }
            self.i += 1;
            return true;
        }
        if self.in_backtick {
            if self.at() == b'`' {
                self.in_backtick = false;
            }
            self.i += 1;
            return true;
        }
        if self.in_bracket {
            if self.at() == b']' {
                self.in_bracket = false;
            }
            self.i += 1;
            return true;
        }
        false
    }

    /// At top level, consumes a `-- line` or `/* block */` comment. Returns
    /// `true` if the cursor sat on a comment opener and was moved past it.
    fn skip_comment(&mut self) -> bool {
        if self.at() == b'-' && self.next() == Some(b'-') {
            while self.more() && self.at() != b'\n' {
                self.i += 1;
            }
            return true;
        }
        if self.at() == b'/' && self.next() == Some(b'*') {
            self.i += 2;
            while self.i + 1 < self.bytes.len()
                && !(self.bytes[self.i] == b'*' && self.bytes[self.i + 1] == b'/')
            {
                self.i += 1;
            }
            self.i = (self.i + 2).min(self.bytes.len());
            return true;
        }
        false
    }

    /// At top level, records entry into a quoting context, or reports a
    /// statement-separating semicolon at the cursor. Advances one byte for a
    /// non-separator character.
    fn enter_delimiter_or_separator(&mut self) -> Option<usize> {
        match self.at() {
            b'\'' => self.in_single = true,
            b'"' => self.in_double = true,
            b'`' => self.in_backtick = true,
            b'[' => self.in_bracket = true,
            b';' => return Some(self.i),
            _ => {}
        }
        self.i += 1;
        None
    }
}
