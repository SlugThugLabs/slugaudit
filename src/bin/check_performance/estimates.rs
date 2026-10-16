//! Criterion estimate collection and baseline loading.
//!
//! Pure I/O + JSON: walks `target/criterion/<group>/<func>/new/estimates.json`
//! to harvest the median point estimate for every bench that just ran,
//! and loads (or rejects) `.planning/archive/perf_baseline.json` for comparison.

use std::collections::BTreeMap;
use std::path::Path;

const CRITERION_DIR: &str = "target/criterion";

pub(crate) fn collect_new_benches() -> std::io::Result<BTreeMap<String, u64>> {
    let dir = Path::new(CRITERION_DIR);
    if !dir.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{CRITERION_DIR} directory does not exist after cargo bench"),
        ));
    }
    collect_benches_in(dir)
}

/// Every `new/estimates.json` under `dir`, keyed `group/func`. Strict on
/// purpose: an unreadable directory, an unreadable or malformed estimates
/// file, or one without a median is an error naming the path — a bench that
/// silently drops out of the harvest would be absent from the gate forever.
pub(crate) fn collect_benches_in(dir: &Path) -> std::io::Result<BTreeMap<String, u64>> {
    let mut paths = Vec::new();
    walk(dir, &mut paths)?;
    let mut out: BTreeMap<String, u64> = Default::default();
    for estimates_path in paths {
        let func_dir = estimates_path.parent().and_then(Path::parent);
        let names = (
            func_dir.and_then(Path::file_name).and_then(|s| s.to_str()),
            func_dir
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|s| s.to_str()),
        );
        let (Some(func_name), Some(group_name)) = names else {
            return Err(invalid(
                &estimates_path,
                "bench directory names are not UTF-8",
            ));
        };
        let raw = std::fs::read_to_string(&estimates_path).map_err(|err| {
            std::io::Error::new(err.kind(), format!("{}: {err}", estimates_path.display()))
        })?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|err| invalid(&estimates_path, &format!("not valid JSON: {err}")))?;
        let median = value
            .get("median")
            .and_then(|m| m.get("point_estimate"))
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| invalid(&estimates_path, "no median.point_estimate"))?;
        out.insert(format!("{group_name}/{func_name}"), median as u64);
    }
    Ok(out)
}

fn invalid(path: &Path, what: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{}: {what}", path.display()),
    )
}

fn walk(dir: &Path, found: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| std::io::Error::new(err.kind(), format!("{}: {err}", dir.display())))?;
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, found)?;
        } else if path.file_name().and_then(|s| s.to_str()) == Some("estimates.json")
            && path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                == Some("new")
        {
            found.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BaselineEntry {
    pub median_ns: u64,
    pub budget_ns: Option<u64>,
}

pub(crate) fn load_baseline(path: &Path) -> Result<BTreeMap<String, BaselineEntry>, String> {
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
