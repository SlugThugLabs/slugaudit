//! The registry of stored results and the admission policy.
//!
//! The map mutex is held for map operations only, never across SQL or file
//! I/O: every entry owns its connection behind its own mutex, and reclaimed
//! entries are handed back so the caller drops them (closing the file) after
//! the map lock is released.
//!
//! Admission invariant: the sum of admitted bytes never exceeds the budget.
//! Transient on-disk bytes while results are still being written are bounded
//! by (concurrent writers x budget), because each writer aborts itself once its
//! own file exceeds the whole budget (`DEVIATIONS.md`).

use super::cursor::ResultId;
use super::error::ResultStoreError;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

/// How a stored result is treated under space pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultClass {
    /// Not delivered and read within the grace period: never reclaimed.
    Active,
    /// Not delivered and older than the grace period: reclaimable, second.
    Idle,
    /// The last page was served: reclaimable first.
    Delivered,
}

/// What a result is, written once at materialisation.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultMeta {
    /// The tool that produced it.
    pub tool: String,
    /// The project label.
    pub project: String,
    /// The index revision it was taken from.
    pub revision_id: String,
    /// Column names.
    pub columns: Vec<String>,
    /// The completeness record the tool supplied.
    pub completeness: serde_json::Value,
    /// Exact number of rows.
    pub row_count: u64,
    /// Unix seconds at materialisation.
    pub created_unix: i64,
}

/// One registered result.
pub(super) struct Entry {
    // Recovering a poisoned lock is safe here: after registration the
    // connection is only read, so a panic mid-read leaves no partial mutation.
    pub(super) conn: Mutex<Connection>,
    pub(super) meta: Arc<ResultMeta>,
    pub(super) bytes: u64,
    seq: u64,
    last_access: AtomicI64,
    delivered: AtomicBool,
}

impl Entry {
    pub(super) fn new(conn: Connection, meta: ResultMeta, bytes: u64, now: i64) -> Self {
        Self {
            conn: Mutex::new(conn),
            meta: Arc::new(meta),
            bytes,
            seq: 0,
            last_access: AtomicI64::new(now),
            delivered: AtomicBool::new(false),
        }
    }

    pub(super) fn class(&self, now: i64, grace_secs: u64) -> ResultClass {
        let idle_for = now.saturating_sub(self.last_access.load(Ordering::Relaxed));
        if self.delivered.load(Ordering::Relaxed) {
            ResultClass::Delivered
        } else if u64::try_from(idle_for).is_ok_and(|idle| idle > grace_secs) {
            ResultClass::Idle
        } else {
            ResultClass::Active
        }
    }

    /// Serving a page renews protection; the last page marks it delivered.
    pub(super) fn served(&self, now: i64, last_page: bool) {
        self.last_access.store(now, Ordering::Relaxed);
        if last_page {
            self.delivered.store(true, Ordering::Relaxed);
        }
    }

    fn age_key(&self) -> (i64, u64) {
        (self.last_access.load(Ordering::Relaxed), self.seq)
    }
}

/// Counters of the store's decisions.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Counters {
    pub(super) reclaimed_delivered: u64,
    pub(super) reclaimed_idle: u64,
    pub(super) refused_full: u64,
    pub(super) refused_too_large: u64,
    pub(super) unknown_cursors: u64,
    pub(super) expired_cursors: u64,
}

/// A result removed to make room; dropped by the caller outside the lock.
pub(super) struct Reclaimed {
    pub(super) id: ResultId,
    pub(super) class: ResultClass,
    pub(super) entry: Arc<Entry>,
}

#[derive(Default)]
pub(super) struct Registry {
    pub(super) entries: HashMap<ResultId, Arc<Entry>>,
    pub(super) admitted_bytes: u64,
    pub(super) counters: Counters,
    next_seq: u64,
}

impl Registry {
    /// Admits `entry` under `budget`, reclaiming delivered results oldest
    /// first, then idle ones oldest first, never active ones. The reclaim set
    /// is planned before anything is removed, so a result that cannot fit
    /// destroys nothing.
    pub(super) fn admit(
        &mut self,
        id: ResultId,
        mut entry: Entry,
        budget: u64,
        now: i64,
        grace_secs: u64,
    ) -> Result<Vec<Reclaimed>, ResultStoreError> {
        if entry.bytes > budget {
            self.counters.refused_too_large += 1;
            tracing::info!(result = %id, bytes = entry.bytes, budget, "result refused: larger than the whole budget");
            return Err(ResultStoreError::TooLarge);
        }
        let plan = self.plan_reclaim(entry.bytes, budget, now, grace_secs)?;
        let mut reclaimed = Vec::with_capacity(plan.len());
        for (victim, class) in plan {
            if let Some(removed) = self.entries.remove(&victim) {
                self.admitted_bytes -= removed.bytes;
                match class {
                    ResultClass::Delivered => self.counters.reclaimed_delivered += 1,
                    _ => self.counters.reclaimed_idle += 1,
                }
                reclaimed.push(Reclaimed {
                    id: victim,
                    class,
                    entry: removed,
                });
            }
        }
        entry.seq = self.next_seq;
        self.next_seq += 1;
        self.admitted_bytes += entry.bytes;
        self.entries.insert(id, Arc::new(entry));
        Ok(reclaimed)
    }

    fn plan_reclaim(
        &mut self,
        bytes: u64,
        budget: u64,
        now: i64,
        grace_secs: u64,
    ) -> Result<Vec<(ResultId, ResultClass)>, ResultStoreError> {
        let mut used = self.admitted_bytes;
        let mut plan = Vec::new();
        if used.saturating_add(bytes) <= budget {
            return Ok(plan);
        }
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .map(|(id, entry)| (*id, entry.class(now, grace_secs), entry))
            .filter(|(_, class, _)| *class != ResultClass::Active)
            .collect();
        // Delivered before idle, each oldest first.
        candidates.sort_by_key(|(_, class, entry)| (*class == ResultClass::Idle, entry.age_key()));
        for (id, class, entry) in candidates {
            used -= entry.bytes;
            plan.push((id, class));
            if used.saturating_add(bytes) <= budget {
                return Ok(plan);
            }
        }
        let active_count = self.count_class(ResultClass::Active, now, grace_secs);
        self.counters.refused_full += 1;
        tracing::info!(
            active_count,
            bytes,
            budget,
            "result refused: the store is full"
        );
        Err(ResultStoreError::Full { active_count })
    }

    pub(super) fn count_class(&self, class: ResultClass, now: i64, grace_secs: u64) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.class(now, grace_secs) == class)
            .count()
    }
}
