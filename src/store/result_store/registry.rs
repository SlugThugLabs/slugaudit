// slugaudit-line-exception: approved-by=split-judge; reason=single result-registry state machine: entry lifecycle classes, space-pressure admission/eviction planning, and StoreStats snapshot read the same Registry fields
//! Registry of stored results, space-pressure admission policy, and stats
//! snapshots.
//!
//! The map mutex is held for map operations only, never across SQL or file I/O:
//! every entry owns its connection behind its own mutex, and reclaimed entries
//! are dropped by the caller after releasing the map lock.

use super::cursor::ResultId;
use super::error::ResultStoreError;
use super::result_file::ResultFile;
use super::sweep::SweepOutcome;
use crate::util::tally;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

/// How a stored result is treated under space pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultClass {
    /// Not delivered and read within the grace period: never reclaimed.
    Active,
    /// Not delivered and older than the grace period: reclaimable second.
    Idle,
    /// Last page already served: reclaimable first.
    Delivered,
}

/// Metadata written once at materialization.
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
    /// Unix seconds at materialization.
    pub created_unix: i64,
}

/// Results and bytes held in one [`ResultClass`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassStats {
    /// Number of results.
    pub results: usize,
    /// Their measured bytes.
    pub bytes: u64,
}

/// Snapshot of what the result store holds and has decided so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreStats {
    /// Active results (never reclaimed).
    pub active: ClassStats,
    /// Idle results (reclaimable second).
    pub idle: ClassStats,
    /// Delivered results (reclaimable first).
    pub delivered: ClassStats,
    /// `max_result_store_bytes`.
    pub budget_bytes: u64,
    /// Sum of admitted bytes; never above the budget.
    pub admitted_bytes: u64,
    /// Delivered results reclaimed so far.
    pub reclaimed_delivered: u64,
    /// Idle results reclaimed so far.
    pub reclaimed_idle: u64,
    /// New results refused with `Full`.
    pub refused_full: u64,
    /// New results refused with `TooLarge`.
    pub refused_too_large: u64,
    /// Cursors refused as unknown.
    pub unknown_cursors: u64,
    /// Cursors refused as expired.
    pub expired_cursors: u64,
    /// What the startup sweep did.
    pub sweep: SweepOutcome,
}

const MAX_OPEN_RESULTS: usize = 64;

pub(super) struct Entry {
    pub(super) conn: Mutex<ResultFile>,
    pub(super) meta: Arc<ResultMeta>,
    pub(super) bytes: u64,
    seq: u64,
    last_access: AtomicI64,
    delivered: AtomicBool,
}

impl Entry {
    pub(super) fn new(conn: ResultFile, meta: ResultMeta, bytes: u64, now: i64) -> Self {
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
        let last_access = self.last_access.load(Ordering::Relaxed);
        let idle_for = (now >= last_access).then_some(now.abs_diff(last_access));
        if self.delivered.load(Ordering::Relaxed) {
            ResultClass::Delivered
        } else if idle_for.is_some_and(|idle| idle > grace_secs) {
            ResultClass::Idle
        } else {
            ResultClass::Active
        }
    }

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

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Counters {
    pub(super) reclaimed_delivered: u64,
    pub(super) reclaimed_idle: u64,
    pub(super) refused_full: u64,
    pub(super) refused_too_large: u64,
    pub(super) unknown_cursors: u64,
    pub(super) expired_cursors: u64,
}

fn fits_budget(used: u64, bytes: u64, budget: u64) -> bool {
    used.checked_add(bytes).is_some_and(|total| total <= budget)
}

pub(super) struct Reclaimed {
    pub(super) id: ResultId,
    pub(super) class: ResultClass,
    pub(super) entry: Arc<Entry>,
}

pub(super) struct Refused {
    pub(super) error: ResultStoreError,
    pub(super) entry: Box<Entry>,
}

#[derive(Default)]
pub(super) struct Registry {
    pub(super) entries: HashMap<ResultId, Arc<Entry>>,
    pub(super) admitted_bytes: u64,
    pub(super) counters: Counters,
    next_seq: u64,
}

impl Registry {
    pub(super) fn admit(
        &mut self,
        id: ResultId,
        mut entry: Entry,
        budget: u64,
        now: i64,
        grace_secs: u64,
    ) -> Result<Vec<Reclaimed>, Refused> {
        if entry.bytes > budget {
            tally(&mut self.counters.refused_too_large);
            return Err(refused(ResultStoreError::TooLarge, entry));
        }
        let plan = match self.plan_reclaim(entry.bytes, budget, now, grace_secs) {
            Ok(plan) => plan,
            Err(error) => return Err(refused(error, entry)),
        };
        let mut reclaimed = Vec::with_capacity(plan.len());
        for (victim, class) in plan {
            if let Some(removed) = self.entries.remove(&victim) {
                let Some(remaining) = self.admitted_bytes.checked_sub(removed.bytes) else {
                    return Err(refused(
                        ResultStoreError::corrupt(
                            "admitted bytes are below a stored result's size",
                        ),
                        entry,
                    ));
                };
                self.admitted_bytes = remaining;
                match class {
                    ResultClass::Delivered => tally(&mut self.counters.reclaimed_delivered),
                    _ => tally(&mut self.counters.reclaimed_idle),
                }
                reclaimed.push(Reclaimed {
                    id: victim,
                    class,
                    entry: removed,
                });
            }
        }
        let (Some(next_seq), Some(admitted_bytes)) = (
            self.next_seq.checked_add(1),
            self.admitted_bytes.checked_add(entry.bytes),
        ) else {
            return Err(refused(
                ResultStoreError::corrupt("result sequence or admitted bytes overflow"),
                entry,
            ));
        };
        entry.seq = self.next_seq;
        self.next_seq = next_seq;
        self.admitted_bytes = admitted_bytes;
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
        let total_entries = self.entries.len();
        let mut plan = Vec::new();
        let fits = |used: u64, evicted: usize| {
            fits_budget(used, bytes, budget)
                && total_entries
                    .checked_sub(evicted)
                    .is_some_and(|n| n < MAX_OPEN_RESULTS)
        };
        if fits(used, 0) {
            return Ok(plan);
        }
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .map(|(id, entry)| (*id, entry.class(now, grace_secs), entry))
            .filter(|(_, class, _)| *class != ResultClass::Active)
            .collect();
        candidates.sort_by_key(|(_, class, entry)| (*class == ResultClass::Idle, entry.age_key()));
        for (id, class, entry) in candidates {
            used = used.checked_sub(entry.bytes).ok_or_else(|| {
                ResultStoreError::corrupt("admitted bytes are below a stored result's size")
            })?;
            plan.push((id, class));
            if fits(used, plan.len()) {
                return Ok(plan);
            }
        }
        let active_count = self.count_class(ResultClass::Active, now, grace_secs);
        tally(&mut self.counters.refused_full);
        Err(ResultStoreError::Full { active_count })
    }

    pub(super) fn count_class(&self, class: ResultClass, now: i64, grace_secs: u64) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.class(now, grace_secs) == class)
            .count()
    }

    pub(super) fn snapshot(
        &self,
        budget_bytes: u64,
        now: i64,
        grace_secs: u64,
        sweep: &SweepOutcome,
    ) -> Result<StoreStats, ResultStoreError> {
        let mut stats = StoreStats {
            budget_bytes,
            admitted_bytes: self.admitted_bytes,
            reclaimed_delivered: self.counters.reclaimed_delivered,
            reclaimed_idle: self.counters.reclaimed_idle,
            refused_full: self.counters.refused_full,
            refused_too_large: self.counters.refused_too_large,
            unknown_cursors: self.counters.unknown_cursors,
            expired_cursors: self.counters.expired_cursors,
            sweep: sweep.clone(),
            ..StoreStats::default()
        };
        for entry in self.entries.values() {
            let slot = match entry.class(now, grace_secs) {
                ResultClass::Active => &mut stats.active,
                ResultClass::Idle => &mut stats.idle,
                ResultClass::Delivered => &mut stats.delivered,
            };
            let (Some(results), Some(bytes)) = (
                slot.results.checked_add(1),
                slot.bytes.checked_add(entry.bytes),
            ) else {
                return Err(ResultStoreError::corrupt("result class totals overflow"));
            };
            slot.results = results;
            slot.bytes = bytes;
        }
        Ok(stats)
    }
}

fn refused(error: ResultStoreError, entry: Entry) -> Refused {
    Refused {
        error,
        entry: Box::new(entry),
    }
}
