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
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut in_bracket = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 1;
                } else {
                    in_single = false;
                }
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'"' {
                if bytes.get(i + 1) == Some(&b'"') {
                    i += 1;
                } else {
                    in_double = false;
                }
            }
            i += 1;
            continue;
        }
        if in_backtick {
            if b == b'`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if in_bracket {
            if b == b']' {
                in_bracket = false;
            }
            i += 1;
            continue;
        }
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'`' => in_backtick = true,
            b'[' => in_bracket = true,
            b';' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}
