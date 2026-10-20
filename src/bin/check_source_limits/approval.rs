//! Parser for source-limit exception comments.

const NEEDLE: &str = "slugaudit-line-exception:";
const AGENT: &str = "approved-by=slugthug;";
const HUMAN: &str = "approved-by=human-user;";
/// Start of the operator-facing reason produced for a human approval.
/// Display only: the line-limit policy keys on [`Approver`], never on this
/// text, because an agent can write this prefix into its own reason.
const HUMAN_APPROVED_PREFIX: &str = "[human-approved ";

/// Which approver an exception annotation names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Approver {
    Agent,
    Human,
}

/// A parsed `slugaudit-line-exception:` annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Exception {
    /// The operator-facing reason. A human approval carries its date here;
    /// nothing in the policy inspects this text.
    pub(super) reason: String,
    approver: Approver,
}

impl Exception {
    /// An agent approval, valid within the original ceiling only.
    pub(super) fn agent(reason: String) -> Self {
        Self {
            reason,
            approver: Approver::Agent,
        }
    }

    /// A dated human approval, the only kind that may authorize the
    /// narrowly bounded 301–350 band.
    pub(super) fn human(date: &str, reason: String) -> Self {
        Self {
            reason: format!("{HUMAN_APPROVED_PREFIX}{date}] {reason}"),
            approver: Approver::Human,
        }
    }

    /// Whether this exception may admit a file above the production ceiling.
    /// This reads the parsed approver, not the reason text.
    pub(super) fn allows_extended_ceiling(&self) -> bool {
        self.approver == Approver::Human
    }
}

/// Returns the parsed exception. Legacy agent approvals remain valid for
/// files within the original ceiling; dated human approvals may authorize
/// the narrowly bounded 301–350 range.
pub(super) fn reason(source: &str) -> Option<Exception> {
    for line in source.lines() {
        let Some(start) = line.find(NEEDLE) else {
            continue;
        };
        let text = line[start + NEEDLE.len()..].trim_start();
        if let Some(rest) = text.strip_prefix(AGENT) {
            return Some(Exception::agent(parse_reason(rest)?));
        }
        if let Some(rest) = text.strip_prefix(HUMAN) {
            let (date, rest) = rest.trim_start().split_once(';')?;
            let date = date.strip_prefix("date=")?.trim();
            if !valid_date(date) {
                continue;
            }
            return Some(Exception::human(date, parse_reason(rest)?));
        }
    }
    None
}

fn parse_reason(text: &str) -> Option<String> {
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
