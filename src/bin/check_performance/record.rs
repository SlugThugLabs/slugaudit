//! `--record`: rewriting the committed baseline from this run.

use std::collections::BTreeMap;
use std::path::Path;

use crate::estimates::BaselineEntry;

/// The baseline document for `measured`, carrying each bench's hand-set
/// `budget_ns` over from `previous` so a re-record never drops a budget.
pub(crate) fn baseline_json(
    measured: &BTreeMap<String, u64>,
    previous: &BTreeMap<String, BaselineEntry>,
    threshold_pct: f64,
    machine: &str,
) -> serde_json::Value {
    let benches: serde_json::Map<String, serde_json::Value> = measured
        .iter()
        .map(|(key, median_ns)| {
            let budget_ns = previous.get(key).and_then(|entry| entry.budget_ns);
            (
                key.clone(),
                serde_json::json!({ "median_ns": median_ns, "budget_ns": budget_ns }),
            )
        })
        .collect();
    serde_json::json!({
        "machine": machine,
        "threshold_percent": threshold_pct,
        "benches": serde_json::Value::Object(benches),
    })
}

/// Writes `baseline` to `path` (creating its directory), newline-terminated.
pub(crate) fn write(path: &Path, baseline: &serde_json::Value) -> Result<(), String> {
    let serialised = serde_json::to_string_pretty(baseline)
        .map_err(|err| format!("could not serialise baseline JSON: {err}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, format!("{serialised}\n"))
        .map_err(|err| format!("could not write baseline {}: {err}", path.display()))
}
