//! Parser for source-limit exception comments.

const NEEDLE: &str = "slugaudit-line-exception:";
const AGENT: &str = "approved-by=slugthuglabs;";
const SPLIT_JUDGE: &str = "approved-by=split-judge;";
const HUMAN: &str = "approved-by=human-user;";
const HUMAN_APPROVED_PREFIX: &str = "[human-approved ";

/// A parsed `slugaudit-line-exception:` annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Exception {
    /// The operator-facing reason.
    pub(super) reason: String,
}

impl Exception {
    pub(super) fn approved(reason: String) -> Self {
        Self { reason }
    }

    pub(super) fn human(date: &str, reason: &str) -> Self {
        Self {
            reason: format!("{HUMAN_APPROVED_PREFIX}{date}] {reason}"),
        }
    }
}

/// Returns the parsed exception (`split-judge`, `slugthuglabs`, or dated
/// `human-user`), if any valid annotation is present.
pub(super) fn reason(source: &str) -> Option<Exception> {
    for line in source.lines() {
        let Some((_, after_needle)) = line.split_once(NEEDLE) else {
            continue;
        };
        let text = after_needle.trim_start();
        // A malformed annotation grants nothing; the scan goes on to the next
        // annotation.
        if let Some(rest) = text
            .strip_prefix(SPLIT_JUDGE)
            .or_else(|| text.strip_prefix(AGENT))
        {
            let Some(reason) = parse_reason(rest) else {
                continue;
            };
            return Some(Exception::approved(reason));
        }
        if let Some(rest) = text.strip_prefix(HUMAN) {
            let Some((date, rest)) = rest.trim_start().split_once(';') else {
                continue;
            };
            let Some(date) = date.strip_prefix("date=").map(str::trim) else {
                continue;
            };
            if !valid_date(date) {
                continue;
            }
            let Some(reason) = parse_reason(rest) else {
                continue;
            };
            return Some(Exception::human(date, &reason));
        }
    }
    None
}

fn parse_reason(text: &str) -> Option<String> {
    let reason = text.trim_start().strip_prefix("reason=")?.trim();
    (!reason.is_empty()).then(|| reason.to_owned())
}

fn valid_date(date: &str) -> bool {
    matches!(
        date.as_bytes(),
        [y1, y2, y3, y4, b'-', m1, m2, b'-', d1, d2]
            if [y1, y2, y3, y4, m1, m2, d1, d2]
                .iter()
                .all(|digit| digit.is_ascii_digit())
    )
}
