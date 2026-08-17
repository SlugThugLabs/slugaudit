//! Checks 3 and 4: plan-status-header freshness and plan-task descope.

use std::path::Path;

use super::read;

/// Check 3: `IMPLEMENTATION_PLAN.md`'s `Status:` line must reference the
/// §22 audit-corrections section; a header reverted to a pre-audit state is
/// caught here.
pub(super) fn check_plan_status_header(root: &Path, failures: &mut Vec<String>) {
    let plan = read(root, ".planning/IMPLEMENTATION_PLAN.md");
    let lines: Vec<&str> = plan.lines().collect();
    let Some(idx) = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Status:"))
    else {
        failures.push("IMPLEMENTATION_PLAN.md has no `Status:` line".to_owned());
        return;
    };
    // The §22 reference may sit on the `Status:` line or its continuation
    // lines (e.g. "Status: … recorded\n(2026-08-10); see §22 for …").
    let paragraph = lines[idx..]
        .iter()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if !paragraph.contains("§22") {
        failures.push(
            "IMPLEMENTATION_PLAN.md `Status:` paragraph does not reference the §22 \
             audit-corrections section; the header predates the audit"
                .to_owned(),
        );
    }
}

/// Check 4: a `### Task X.Y` section whose listed `src/` files all fail to
/// exist must have an explicit descope entry in `DECISIONS.md` mentioning
/// that exact task id (`Task X.Y`). This is the plan's own requirement:
/// "plan phases that list files with no implementation and no
/// `DECISIONS.md` descope entry."
pub(super) fn check_plan_task_descope(root: &Path, failures: &mut Vec<String>) {
    let plan = read(root, ".planning/IMPLEMENTATION_PLAN.md");
    let decisions = read(root, ".planning/DECISIONS.md");
    let lines: Vec<&str> = plan.lines().collect();

    // Split into `### Task X.Y` sections.
    let mut task_starts: Vec<usize> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if let Some(id) = task_id_from_header(line)
            && id.starts_with("Task ")
        {
            task_starts.push(idx);
        }
    }

    for (slot, start) in task_starts.iter().enumerate() {
        let end = task_starts.get(slot + 1).copied().unwrap_or(lines.len());
        let header = lines[*start];
        let Some(task_id) = task_id_from_header(header) else {
            continue;
        };
        let section = lines[*start + 1..end].join("\n");

        // Collect `- \`src/...\`` file bullets.
        let src_files: Vec<&str> = section
            .lines()
            .filter_map(|l| {
                let trimmed = l.trim_start();
                let rest = trimmed.strip_prefix("- `")?;
                let path = rest.split('`').next()?;
                path.strip_prefix("src/").map(|_| path)
            })
            .collect();
        if src_files.is_empty() {
            continue; // task lists no src files — nothing to verify
        }

        let any_exists = src_files.iter().any(|f| root.join(f).exists());
        if any_exists {
            continue; // implemented
        }

        // All listed files are missing: require a descope entry mentioning the
        // exact task id in DECISIONS.md.
        if !decisions.contains(&task_id) {
            failures.push(format!(
                "{header} lists only missing files ({}) and DECISIONS.md has no descope entry \
                 mentioning `{task_id}`; either implement it or record the descope decision",
                src_files.join(", ")
            ));
        }
    }
}

pub(super) fn task_id_from_header(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("### ")?;
    let mut parts = rest.split_whitespace();
    let word = parts.next()?;
    let id = parts.next()?;
    let valid_id = id.split('.').count() == 2 && id.chars().all(|c| c.is_ascii_digit() || c == '.');
    if word == "Task" && valid_id {
        Some(format!("{word} {id}"))
    } else {
        None
    }
}
