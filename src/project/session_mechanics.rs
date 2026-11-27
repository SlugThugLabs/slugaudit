//! Auditor-registered session mechanics, keyed per canonical project root and
//! held in memory only for the lifetime of this server process.

use crate::util::LockPoisoned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One auditor-registered mechanic: a Tree-sitter query with a single capture
/// run over indexed files of `language`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicEntry {
    /// Identifier `leads` accepts as `mechanic`; never a built-in mechanic id.
    pub id: String,
    /// Grammar name the query compiles against and scans.
    pub language: String,
    /// Tree-sitter S-expression with a single capture.
    pub query: String,
    /// The model's note describing what the mechanic counts.
    pub note: String,
}

/// Validated [`MechanicEntry`] paired with its pre-compiled Tree-sitter [`tree_sitter::Query`].
#[derive(Debug, Clone)]
pub struct CompiledMechanic {
    /// Wire metadata of the registered mechanic.
    pub entry: MechanicEntry,
    /// Compiled Tree-sitter query reused across `leads` scans.
    pub compiled: Arc<tree_sitter::Query>,
}

/// Most session mechanics one project may register; a 65th `add` is refused.
pub const MAX_SESSION_MECHANICS: usize = 64;

#[derive(Debug, Default)]
struct MechanicsState {
    by_project: HashMap<PathBuf, Vec<CompiledMechanic>>,
    report_notes_delivered: HashSet<PathBuf>,
}

/// In-memory session mechanics shared across clones of `SourceSyncManager`,
/// scoped per canonical project root and wiped when the process exits or a
/// project is turned `off`.
#[derive(Debug, Clone, Default)]
pub struct SessionMechanics {
    state: Arc<Mutex<MechanicsState>>,
}

impl SessionMechanics {
    /// Snapshot of registered mechanics for `project_root`, oldest first.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the session-mechanics mutex was poisoned.
    pub fn entries(&self, project_root: &Path) -> Result<Vec<CompiledMechanic>, LockPoisoned> {
        let state = crate::util::lock(&self.state, "session_mechanics")?;
        let Some(entries) = state.by_project.get(project_root) else {
            return Ok(Vec::new());
        };
        Ok(entries.clone())
    }

    /// Runs `change` against `project_root`'s mechanics list under the lock so
    /// collision checks, cap enforcement, and mutation are atomic.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the session-mechanics mutex was poisoned.
    pub fn with_entries<T>(
        &self,
        project_root: &Path,
        change: impl FnOnce(&mut Vec<CompiledMechanic>) -> T,
    ) -> Result<T, LockPoisoned> {
        let mut state = crate::util::lock(&self.state, "session_mechanics")?;
        let entries = state
            .by_project
            .entry(project_root.to_path_buf())
            .or_default();
        Ok(change(entries))
    }

    /// Clears all custom mechanics and resets the `report` note state for
    /// `project_root` (called on `project_control off`).
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the session-mechanics mutex was poisoned.
    pub fn clear_project(&self, project_root: &Path) -> Result<(), LockPoisoned> {
        let mut state = crate::util::lock(&self.state, "session_mechanics")?;
        state.by_project.remove(project_root);
        state.report_notes_delivered.remove(project_root);
        Ok(())
    }

    /// True once per `project_root` (until `clear_project`) for the first
    /// `report` call that delivers the mechanics notice.
    ///
    /// # Errors
    ///
    /// Returns `LockPoisoned` when the session-mechanics mutex was poisoned.
    pub fn claim_report_note(&self, project_root: &Path) -> Result<bool, LockPoisoned> {
        let mut state = crate::util::lock(&self.state, "session_mechanics")?;
        Ok(state
            .report_notes_delivered
            .insert(project_root.to_path_buf()))
    }
}

#[cfg(test)]
#[path = "session_mechanics_tests.rs"]
mod tests;
