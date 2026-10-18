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
    let mut causes: Vec<String> = Vec::new();
    match std::fs::read_to_string("/proc/meminfo") {
        Ok(content) => match parse_proc_meminfo(&content) {
            Ok(info) => return info,
            Err(reason) => causes.push(format!("/proc/meminfo: {reason}")),
        },
        Err(error) => causes.push(format!("/proc/meminfo: {error}")),
    }

    #[cfg(target_os = "macos")]
    {
        match sysctl_memsize() {
            Ok(total) => {
                return SystemMemoryInfo {
                    total_bytes: total,
                    available_bytes: total / 2,
                };
            }
            Err(reason) => causes.push(format!("sysctl hw.memsize: {reason}")),
        }
    }

    tracing::warn!(
        causes = %causes.join("; "),
        "could not detect system memory; sizing the adaptive profile from the conservative 8 GiB default"
    );
    SystemMemoryInfo {
        total_bytes: 8 * 1024 * 1024 * 1024,
        available_bytes: 4 * 1024 * 1024 * 1024,
    }
}

/// Runs `sysctl -n hw.memsize`, returning the failure cause as text.
#[cfg(target_os = "macos")]
fn sysctl_memsize() -> Result<u64, String> {
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .map_err(|error| format!("could not run sysctl: {error}"))?;
    if !output.status.success() {
        return Err(format!("sysctl exited with {}", output.status));
    }
    parse_sysctl_memsize(&output.stdout)
}

#[cfg(any(target_os = "macos", test))]
fn parse_sysctl_memsize(stdout: &[u8]) -> Result<u64, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|error| format!("output is not valid UTF-8: {error}"))?;
    text.trim()
        .parse::<u64>()
        .map_err(|error| format!("output is not a byte count: {error}"))
}

fn parse_proc_meminfo(content: &str) -> Result<SystemMemoryInfo, String> {
    let mut total_kb = None;
    let mut avail_kb = None;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = Some(parse_kb(line)?);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = Some(parse_kb(line)?);
        }
        if total_kb.is_some() && avail_kb.is_some() {
            break;
        }
    }
    let total_bytes = kb_to_bytes(total_kb.ok_or("no MemTotal line")?)?;
    let available_bytes = match avail_kb {
        Some(kb) => kb_to_bytes(kb)?,
        None => total_bytes / 2,
    };
    Ok(SystemMemoryInfo {
        total_bytes,
        available_bytes,
    })
}

fn kb_to_bytes(kb: u64) -> Result<u64, String> {
    kb.checked_mul(1024)
        .ok_or_else(|| format!("{kb} kB overflows a byte count"))
}

fn parse_kb(line: &str) -> Result<u64, String> {
    let field = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("no value on line '{line}'"))?;
    field
        .parse()
        .map_err(|error| format!("malformed value '{field}' on line '{line}': {error}"))
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
        assert!(parse_proc_meminfo(sample).is_err());
        assert!(parse_proc_meminfo("invalid content").is_err());
    }

    #[test]
    fn a_malformed_memtotal_line_names_the_line_in_its_error() {
        let error = parse_proc_meminfo("MemTotal:  lots kB\n").expect_err("malformed value");

        assert!(error.contains("malformed value 'lots'"), "{error}");
    }

    #[test]
    fn a_memtotal_that_overflows_bytes_is_an_error_not_a_panic() {
        let sample = format!("MemTotal: {} kB\n", u64::MAX);

        let error = parse_proc_meminfo(&sample).expect_err("overflow");

        assert!(error.contains("overflows"), "{error}");
    }

    #[test]
    fn a_sysctl_output_that_is_not_a_number_is_an_error_with_a_cause() {
        assert_eq!(parse_sysctl_memsize(b"17179869184\n"), Ok(17_179_869_184));
        assert!(parse_sysctl_memsize(b"n/a").is_err());
        assert!(parse_sysctl_memsize(&[0xff]).is_err());
    }

    #[test]
    fn detect_returns_positive_memory() {
        let info = detect_system_memory();
        assert!(info.total_bytes > 0);
        assert!(info.available_bytes > 0);
        assert!(info.available_bytes <= info.total_bytes);
    }
}
