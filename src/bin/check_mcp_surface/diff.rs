//! Structural diff between the committed snapshot and the live surface.

use serde_json::Value;

/// Longest leaf value printed in a failure line; a reworded description is
/// reported by path, not by reprinting the whole description.
const MAX_LEAF_CHARS: usize = 60;

/// Every difference between the committed surface and the live one, as
/// `path: detail` lines. An empty vector means no drift.
///
/// Arrays whose entries are all `{"name": …}` objects — the tool and the
/// resource lists — are matched by name, so reordering the catalogue is not
/// drift. Every other array (a JSON Schema's `required`, an `enum`, …) is
/// compared by position.
pub(super) fn differences(committed: &Value, live: &Value) -> Vec<String> {
    let mut found = Vec::new();
    walk(committed, live, "", &mut found);
    found
}

fn walk(committed: &Value, live: &Value, path: &str, found: &mut Vec<String>) {
    match (committed, live) {
        (Value::Object(committed), Value::Object(live)) => {
            for (key, value) in committed {
                let child = join(path, key);
                match live.get(key) {
                    Some(other) => walk(value, other, &child, found),
                    None => found.push(format!("{child}: in the snapshot, absent now")),
                }
            }
            for key in live.keys() {
                if !committed.contains_key(key) {
                    found.push(format!(
                        "{}: absent from the snapshot, present now",
                        join(path, key)
                    ));
                }
            }
        }
        (Value::Array(committed), Value::Array(live)) => {
            if !named(committed) || !named(live) {
                if committed.len() != live.len() {
                    found.push(format!(
                        "{path}: snapshot has {} entries, live has {}",
                        committed.len(),
                        live.len()
                    ));
                }
                for (index, (value, other)) in committed.iter().zip(live.iter()).enumerate() {
                    walk(value, other, &format!("{path}[{index}]"), found);
                }
                return;
            }
            for entry in committed {
                let Some(name) = name_of(entry) else { continue };
                match live.iter().find(|other| name_of(other) == Some(name)) {
                    Some(other) => walk(entry, other, &format!("{path}[{name}]"), found),
                    None => found.push(format!("{path}[{name}]: in the snapshot, absent now")),
                }
            }
            for entry in live {
                let Some(name) = name_of(entry) else { continue };
                if !committed.iter().any(|other| name_of(other) == Some(name)) {
                    found.push(format!(
                        "{path}[{name}]: absent from the snapshot, present now"
                    ));
                }
            }
        }
        _ if committed != live => found.push(format!(
            "{path}: snapshot {}, live {}",
            summarize(committed),
            summarize(live)
        )),
        _ => {}
    }
}

/// True when the array is non-empty and every entry is an object with a
/// string `name` — the shape the tool and resource lists use.
fn named(entries: &[Value]) -> bool {
    !entries.is_empty() && entries.iter().all(|entry| name_of(entry).is_some())
}

/// `a.b`, or just `b` at the root so the first reported path has no leading
/// dot.
fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn name_of(entry: &Value) -> Option<&str> {
    entry.get("name")?.as_str()
}

/// A leaf value for a failure line, truncated so one reworded description
/// cannot fill the output.
fn summarize(value: &Value) -> String {
    let rendered = value.to_string();
    let mut chars = rendered.chars();
    let head: String = chars.by_ref().take(MAX_LEAF_CHARS).collect();
    if chars.next().is_some() {
        return format!("{head}…");
    }
    head
}
