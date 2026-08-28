//! Parser for source-limit exception comments.

const NEEDLE: &str = "slugaudit-line-exception:";
const AGENT: &str = "approved-by=agent;";
const HUMAN: &str = "approved-by=human-user;";

/// Returns the exception reason. Legacy agent approvals remain valid for
/// files within the original ceiling; dated human approvals may authorize
/// the narrowly bounded 301–350 range.
pub(super) fn reason(source: &str) -> Option<String> {
    for line in source.lines() {
        let Some(start) = line.find(NEEDLE) else {
            continue;
        };
        let text = line[start + NEEDLE.len()..].trim_start();
        if let Some(rest) = text.strip_prefix(AGENT) {
            return parse_reason(rest, false);
        }
        if let Some(rest) = text.strip_prefix(HUMAN) {
            let (date, rest) = rest.trim_start().split_once(';')?;
            let date = date.strip_prefix("date=")?.trim();
            if !valid_date(date) {
                continue;
            }
            let reason = parse_reason(rest, true)?;
            return Some(format!("[human-approved {date}] {reason}"));
        }
    }
    None
}

fn parse_reason(text: &str, _approved: bool) -> Option<String> {
    let reason = text.trim_start().strip_prefix("reason=")?.trim();
    (!reason.is_empty()).then(|| reason.to_owned())
}

fn valid_date(date: &str) -> bool {
    date.len() == 10
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
}
