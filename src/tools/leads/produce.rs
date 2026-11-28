//! Read-transaction producer for a fresh `leads` call: runs custom-mechanic
//! scans, built-in SQL counts, untested-language notices, and row streaming.

use super::coverage::{files_not_parsed, languages_read};
use super::custom::{self as leads_custom, CustomScan, index_out_of_range};
use super::mechanics::Selected;
use super::query::{self as leads_query, compact_scope, count_all, stream_compact};
use super::stored::{Extras, Feed, to_i64};
use super::{LeadMode, MechanicCount};
use crate::tools::mechanic::catalog::Catalog;
use crate::tools::paginate::{Produced, RowSink};
use crate::tools::untested_language_notices;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::collections::BTreeSet;
use std::time::Instant;

/// Everything a fresh `leads` request asks of the read-transaction producer.
pub(in crate::tools) struct Scope<'a> {
    pub(in crate::tools) catalog: &'a Catalog,
    pub(in crate::tools) selected: Option<Selected>,
    pub(in crate::tools) mode: LeadMode,
    pub(in crate::tools) pattern: Option<&'a str>,
    pub(in crate::tools) deadline: Option<Instant>,
}

pub(super) fn produce(
    tx: &Transaction<'_>,
    sink: &mut RowSink<'_, '_>,
    scope: &Scope<'_>,
    limit: Option<usize>,
) -> Result<Produced<Extras>, ErrorData> {
    let glob_ref = scope.pattern;
    let read = languages_read(tx, glob_ref)?;
    let scanned = scanned_entries(scope);
    let retain_rows = matches!(
        (scope.mode, scope.selected),
        (_, Some(Selected::Registered(_))) | (LeadMode::Compact, None)
    );
    let found = leads_custom::scan_entries(
        tx,
        scope.catalog,
        &scanned,
        retain_rows,
        glob_ref,
        scope.deadline,
        &read,
    )?;
    let mut counts = found.counts.clone();
    counts.extend(count_all(tx, glob_ref, &read)?);
    let notices = notices_for(scope, &counts)?;
    let files_not_parsed = files_not_parsed(tx, glob_ref)?;
    let mut feed = Feed::new(sink);
    stream_selected(tx, scope, glob_ref, &found, &mut feed)?;
    Ok(Produced {
        completeness: found.completeness,
        extra: Extras {
            counts,
            notices,
            files_not_parsed,
            files_total: to_i64(feed.files())?,
            compact: scope.mode == LeadMode::Compact,
            limit,
        },
    })
}

fn stream_selected(
    tx: &Transaction<'_>,
    scope: &Scope<'_>,
    glob: Option<&str>,
    found: &CustomScan,
    feed: &mut Feed<'_, '_, '_>,
) -> Result<(), ErrorData> {
    match (scope.mode, scope.selected) {
        (LeadMode::Full, None) => {}
        (LeadMode::Full, Some(Selected::Builtin(mechanic))) => {
            leads_query::stream_rows(tx, mechanic, glob, &mut |row| feed.full(row))?;
        }
        (LeadMode::Compact, Some(Selected::Builtin(mechanic))) => {
            let only = compact_scope(Some(mechanic));
            stream_compact(tx, &only, glob, &mut |row| feed.compact(row))?;
        }
        (LeadMode::Compact, None) => {
            for index in 0..scope.catalog.custom.len() {
                found.feed_entry(scope.catalog, index, scope.mode, feed)?;
            }
            let all = compact_scope(None);
            stream_compact(tx, &all, glob, &mut |row| feed.compact(row))?;
        }
        (_, Some(Selected::Registered(index))) => {
            found.feed_entry(scope.catalog, index, scope.mode, feed)?;
        }
    }
    Ok(())
}

fn notices_for(scope: &Scope<'_>, counts: &[MechanicCount]) -> Result<Vec<String>, ErrorData> {
    let selected_id = match scope.selected {
        None => None,
        Some(Selected::Builtin(mechanic)) => Some(mechanic.id),
        Some(Selected::Registered(index)) => {
            let registered =
                scope.catalog.custom.get(index).ok_or_else(|| {
                    index_out_of_range("custom", index, scope.catalog.custom.len())
                })?;
            Some(registered.entry.id.as_str())
        }
    };
    let languages: BTreeSet<&str> = counts
        .iter()
        .filter(|count| selected_id.is_none_or(|id| count.mechanic == id))
        .flat_map(|count| count.covers.iter().map(String::as_str))
        .collect();
    Ok(untested_language_notices(
        languages.into_iter().map(|language| (language, None)),
    ))
}

fn scanned_entries(scope: &Scope<'_>) -> Vec<usize> {
    match scope.selected {
        None => (0..scope.catalog.custom.len()).collect(),
        Some(Selected::Builtin(_)) => Vec::new(),
        Some(Selected::Registered(index)) => vec![index],
    }
}
