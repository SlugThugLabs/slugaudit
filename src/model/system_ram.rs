//! Host memory detection for dynamic resource budgeting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Host memory as measured by `detect_system_memory`. Surfaced in the `health` response because it sizes the Adaptive
/// profile's import budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemMemoryInfo {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Memory available to this process for import budgets, in bytes.
    pub available_bytes: u64,
}

/// Host memory could not be measured by any supported source.
#[derive(Debug, thiserror::Error)]
#[error("system memory could not be detected: {causes}")]
pub struct MemoryDetectError {
    /// One entry per source tried, with why it failed.
    causes: String,
}

/// Detects host physical and available memory.
///
/// Tries `/proc/meminfo` (Linux) and `sysctl hw.memsize` (macOS) in turn.
/// When neither yields a value this fails closed with every cause: the
/// Adaptive profile is never sized off a guessed machine, and the caller
/// (startup, `health`) reports the memory as undetected. The `/proc` parse
/// is kept rather than `sysconf`/`statfs` because no `libc` dependency
/// exists and adding one is outside this change.
///
/// # Errors
///
/// Returns [`MemoryDetectError`] when no source yields a value.
pub fn detect_system_memory() -> Result<SystemMemoryInfo, MemoryDetectError> {
    detect_from(std::fs::read_to_string("/proc/meminfo"))
}

/// [`detect_system_memory`] with the `/proc/meminfo` read supplied, so the
/// failure path is testable on a host where the file exists.
fn detect_from(meminfo: std::io::Result<String>) -> Result<SystemMemoryInfo, MemoryDetectError> {
    let mut causes: Vec<String> = Vec::new();
    match meminfo {
        Ok(content) => match parse_proc_meminfo(&content) {
            Ok(info) => return Ok(info),
            Err(reason) => causes.push(format!("/proc/meminfo: {reason}")),
        },
        Err(error) => causes.push(format!("/proc/meminfo: {error}")),
    }

    #[cfg(target_os = "macos")]
    {
        match sysctl_memsize() {
            Ok(total) => {
                return Ok(SystemMemoryInfo {
                    total_bytes: total,
                    available_bytes: total / 2,
                });
            }
            Err(reason) => causes.push(format!("sysctl hw.memsize: {reason}")),
        }
    }

    Err(MemoryDetectError {
        causes: causes.join("; "),
    })
}

/// A detection failure for tests of the callers' fail-closed paths.
#[cfg(test)]
pub(super) fn detect_from_for_test() -> Result<SystemMemoryInfo, MemoryDetectError> {
    Err(MemoryDetectError {
        causes: "test: no memory source".to_owned(),
    })
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
        let info = detect_system_memory().expect("this host reports its memory");
        assert!(info.total_bytes > 0);
        assert!(info.available_bytes > 0);
        assert!(info.available_bytes <= info.total_bytes);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn fail_closed_undetectable_memory_is_an_error_naming_the_cause() {
        let unreadable = std::io::Error::new(std::io::ErrorKind::NotFound, "no meminfo here");

        let error = detect_from(Err(unreadable)).expect_err("no source yields a value");

        assert_eq!(
            error.to_string(),
            "system memory could not be detected: /proc/meminfo: no meminfo here"
        );
    }
}
