//! Token scanner state machine for source limit checking.

pub(crate) enum State {
    Normal,
    LineComment,
    BlockComment,
    String,
    Char,
    RawString,
}

pub(crate) fn scan_step(
    bytes: &[u8],
    source: &str,
    index: &mut usize,
    state: &mut State,
    has_code: &mut bool,
    count: &mut usize,
    block_depth: &mut usize,
    raw_hashes: &mut usize,
) {
    let len = bytes.len();
    match state {
        State::LineComment => {
            if bytes[*index] == b'\n' {
                if *has_code {
                    *count += 1;
                    *has_code = false;
                }
                *state = State::Normal;
            }
            *index += 1;
        }
        State::BlockComment => {
            if *index + 1 < len && bytes[*index] == b'/' && bytes[*index + 1] == b'*' {
                *block_depth += 1;
                *index += 2;
            } else if *index + 1 < len && bytes[*index] == b'*' && bytes[*index + 1] == b'/' {
                *block_depth -= 1;
                if *block_depth == 0 {
                    *state = State::Normal;
                }
                *index += 2;
            } else if bytes[*index] == b'\n' {
                if *has_code {
                    *count += 1;
                    *has_code = false;
                }
                *index += 1;
            } else {
                *index += 1;
            }
        }
        State::String => {
            *has_code = true;
            let byte = bytes[*index];
            if byte == b'\\' && *index + 1 < len {
                *index += 2;
            } else if byte == b'"' {
                *state = State::Normal;
                *index += 1;
            } else if byte == b'\n' {
                if *has_code {
                    *count += 1;
                    *has_code = false;
                }
                *index += 1;
            } else {
                *index += 1;
            }
        }
        State::Char => {
            *has_code = true;
            let byte = bytes[*index];
            if byte == b'\\' && *index + 1 < len {
                *index += 2;
            } else if byte == b'\'' {
                *state = State::Normal;
                *index += 1;
            } else if byte == b'\n' {
                if *has_code {
                    *count += 1;
                    *has_code = false;
                }
                *index += 1;
            } else {
                *index += 1;
            }
        }
        State::RawString => {
            *has_code = true;
            let byte = bytes[*index];
            if byte == b'"' && *index + *raw_hashes < len {
                let all_hashes = (1..=*raw_hashes).all(|i| bytes[*index + i] == b'#');
                if all_hashes {
                    *state = State::Normal;
                    *index += *raw_hashes + 1;
                    return;
                }
            }
            if byte == b'\n' {
                if *has_code {
                    *count += 1;
                    *has_code = false;
                }
                *index += 1;
            } else {
                *index += 1;
            }
        }
        State::Normal => {
            let byte = bytes[*index];
            if byte == b'\n' {
                if *has_code {
                    *count += 1;
                    *has_code = false;
                }
                *index += 1;
                return;
            }
            if *index + 1 < len && byte == b'/' && bytes[*index + 1] == b'/' {
                *state = State::LineComment;
                *index += 2;
                return;
            }
            if *index + 1 < len && byte == b'/' && bytes[*index + 1] == b'*' {
                *state = State::BlockComment;
                *block_depth = 1;
                *index += 2;
                return;
            }
            if byte == b'"' {
                *state = State::String;
                *has_code = true;
                *index += 1;
                return;
            }
            if byte == b'\'' && is_char_literal_start(&source[*index..]) {
                *state = State::Char;
                *has_code = true;
                *index += 1;
                return;
            }
            if byte == b'r'
                && let Some(n) = is_raw_string_start(&source[*index..])
            {
                *state = State::RawString;
                *raw_hashes = n;
                *has_code = true;
                *index += 1 + n + 1;
                return;
            }
            if !byte.is_ascii_whitespace() {
                *has_code = true;
            }
            *index += 1;
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
    if bytes.is_empty() || bytes[0] != b'r' {
        return None;
    }
    let mut i = 1;
    let mut n_hashes = 0;
    while i < bytes.len() && bytes[i] == b'#' {
        n_hashes += 1;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'"' {
        Some(n_hashes)
    } else {
        None
    }
}
