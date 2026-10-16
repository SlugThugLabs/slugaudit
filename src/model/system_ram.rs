//! Host memory detection for dynamic resource budgeting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Host memory as measured at startup (or the conservative fallback,
/// reported by `detect_system_memory` when it could not be measured).
/// Surfaced in the `health` response because it sizes the Adaptive
/// profile's import budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemMemoryInfo {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Memory available to this process for import budgets, in bytes.
    pub available_bytes: u64,
}

/// Detects host physical and available memory.
///
/// Tries `/proc/meminfo` (Linux) and `sysctl hw.memsize` (macOS) in turn.
/// When neither yields a value, the conservative 8 GiB / 4 GiB floor is
/// returned — small enough to be safe on any host, and **warned**, because
/// sizing the Adaptive profile off a guessed value is exactly the kind of
/// silent operator-visible effect this project reports rather than swallows.
#[must_use]
pub fn detect_system_memory() -> SystemMemoryInfo {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo")
        && let Some(info) = parse_proc_meminfo(&content)
    {
        return info;
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
        {
            if let Ok(s) = std::str::from_utf8(&output.stdout) {
                if let Ok(total) = s.trim().parse::<u64>() {
                    return SystemMemoryInfo {
                        total_bytes: total,
                        available_bytes: total / 2,
                    };
                }
            }
        }
    }

    tracing::warn!(
        "could not detect system memory; sizing the adaptive profile from the conservative 8 GiB default"
    );
    SystemMemoryInfo {
        total_bytes: 8 * 1024 * 1024 * 1024,
        available_bytes: 4 * 1024 * 1024 * 1024,
    }
}

fn parse_proc_meminfo(content: &str) -> Option<SystemMemoryInfo> {
    let mut total_kb = None;
    let mut avail_kb = None;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = parse_kb(line);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = parse_kb(line);
        }
        if total_kb.is_some() && avail_kb.is_some() {
            break;
        }
    }
    let total_bytes = total_kb? * 1024;
    let avail_bytes = avail_kb.map_or(total_bytes / 2, |k| k * 1024);
    Some(SystemMemoryInfo {
        total_bytes,
        available_bytes: avail_bytes,
    })
}

fn parse_kb(line: &str) -> Option<u64> {
    line.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proc_meminfo_reads_valid_data() {
        let sample = "MemTotal:       16384000 kB\nMemFree:         8192000 kB\nMemAvailable:   12288000 kB\n";
        let info = parse_proc_meminfo(sample).expect("parse succeeds");
        assert_eq!(info.total_bytes, 16_384_000 * 1024);
        assert_eq!(info.available_bytes, 12_288_000 * 1024);
    }

    #[test]
    fn parse_proc_meminfo_handles_missing_memavailable() {
        let sample = "MemTotal:       16384000 kB\nMemFree:         8192000 kB\n";
        let info = parse_proc_meminfo(sample).expect("parse succeeds");
        assert_eq!(info.total_bytes, 16_384_000 * 1024);
        assert_eq!(info.available_bytes, 8_192_000 * 1024);
    }

    #[test]
    fn parse_proc_meminfo_handles_missing_memtotal() {
        let sample = "MemFree:         8192000 kB\nMemAvailable:   12288000 kB\n";
        assert!(parse_proc_meminfo(sample).is_none());
        assert!(parse_proc_meminfo("invalid content").is_none());
    }

    #[test]
    fn detect_returns_positive_memory() {
        let info = detect_system_memory();
        assert!(info.total_bytes > 0);
        assert!(info.available_bytes > 0);
        assert!(info.available_bytes <= info.total_bytes);
    }
}
