//! The reconciliation policy: resource budget, shared deadline, and
//! ignore rules. Distinct from the working set a pipeline call operates
//! on (connection, root, dirty/deleted sets, expected revision) so
//! callers can't mix up budget and rule concerns.

use crate::ignore_rules::IgnoreRules;
use crate::model::ResourceLimits;
use crate::model::process_limits;
use crate::util::Deadline;
use std::path::Path;
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
    ///
    /// # Errors
    ///
    /// Fails when the project's `config.json` is present but unreadable or
    /// malformed, rather than syncing under limits the file did not say.
    pub(crate) fn for_sync(
        root: &Path,
        rules: Option<Arc<IgnoreRules>>,
    ) -> Result<Self, super::ReconcileError> {
        // Full publishes apply the project config before sampling. Reconcile
        // must derive its limits the same way or an edit can be accepted or
        // skipped solely according to which freshness path happened to run.
        let limits = crate::project::ProjectConfig::limits_for(root, *process_limits())?;
        let deadline = Deadline::after(limits.max_sync_wall_clock);
        Ok(Self {
            limits,
            deadline,
            rules,
        })
    }
}
