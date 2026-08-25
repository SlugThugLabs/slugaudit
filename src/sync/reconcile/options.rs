//! The reconciliation policy: resource budget, shared deadline, and
//! ignore rules. Distinct from the working set a pipeline call operates
//! on (connection, root, dirty/deleted sets, expected revision) so
//! callers can't mix up budget and rule concerns.

use crate::ignore_rules::IgnoreRules;
use crate::model::ResourceLimits;
use crate::model::process_limits;
use crate::util::Deadline;
use std::sync::Arc;

/// The reconciliation context every dirty path is checked against,
/// distinct from the working set (connection, root, dirty, deleted,
/// expected revision): the resource budget, the shared deadline, and the
/// project's ignore rules. Grouped so callers can't mix up budget and
/// rule concerns and to keep the function signature readable.
pub(crate) struct ReconcileOptions {
    pub limits: ResourceLimits,
    pub deadline: Deadline,
    pub rules: Option<Arc<IgnoreRules>>,
}

impl ReconcileOptions {
    /// Options for a production sync pass: the standard resource budget
    /// and deadline, plus the project's current ignore rules.
    pub fn for_sync(rules: Option<Arc<IgnoreRules>>) -> Self {
        let limits = *process_limits();
        let deadline = Deadline::after(limits.max_sync_wall_clock);
        Self {
            limits,
            deadline,
            rules,
        }
    }
}
