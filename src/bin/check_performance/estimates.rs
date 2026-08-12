//! Criterion estimate collection and baseline loading.
//!
//! Pure I/O + JSON: walks `target/criterion/<group>/<func>/new/estimates.json`
//! to harvest the median point estimate for every bench that just ran,
//! and loads (or rejects) `.planning/perf_baseline.json` for comparison.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CRITERION_DIR: &str = "target/criterion";

pub fn collect_new_benches() -> std::io::Result<BTreeMap<String, u64>> {
    let dir = Path::new(CRITERION_DIR);
    if !dir.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{CRITERION_DIR} directory does not exist after cargo bench"),
        ));
    }
    let mut out: BTreeMap<String, u64> = Default::default();
    walk(dir, &mut |estimates_path| {
        let func_dir = estimates_path.parent().and_then(Path::parent);
        let group_dir = func_dir.and_then(Path::parent);
        let Some(func_name) = func_dir
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
        else {
            return;
        };
        let Some(group_name) = group_dir
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
        else {
            return;
        };
        let key = format!("{group_name}/{func_name}");
        let raw = match std::fs::read_to_string(estimates_path) {
            Ok(s) => s,
            Err(_) => return,
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return,
        };
        let median = value
            .get("median")
            .and_then(|m| m.get("point_estimate"))
            .and_then(serde_json::Value::as_u64);
        if let Some(median) = median {
            out.insert(key, median);
        }
    });
    Ok(out)
}

fn walk(dir: &Path, on_estimates: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, on_estimates);
        } else if path.file_name().and_then(|s| s.to_str()) == Some("estimates.json")
            && path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                == Some("new")
        {
            on_estimates(&path);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BaselineEntry {
    pub median_ns: u64,
    pub budget_ns: Option<u64>,
}

pub fn load_baseline(path: &Path) -> Result<BTreeMap<String, BaselineEntry>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read baseline {}: {err}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|err| format!("could not parse baseline JSON: {err}"))?;
    let mut map: BTreeMap<String, BaselineEntry> = BTreeMap::new();
    let Some(benches) = value.get("benches").and_then(serde_json::Value::as_object) else {
        return Err("baseline JSON missing 'benches' object".to_string());
    };
    for (key, entry) in benches {
        let median_ns = entry
            .get("median_ns")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("baseline entry {key:?} missing median_ns"))?;
        let budget_ns = entry.get("budget_ns").and_then(serde_json::Value::as_u64);
        map.insert(
            key.clone(),
            BaselineEntry {
                median_ns,
                budget_ns,
            },
        );
    }
    Ok(map)
}

// Stub to silence the unused-import warning for `PathBuf` when the
// baseline entry struct only appears behind pinned JSON paths.
#[allow(dead_code)]
fn _stub_pathbuf(_: PathBuf) {}
