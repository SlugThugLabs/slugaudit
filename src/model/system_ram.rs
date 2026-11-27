//! Host memory detection for dynamic resource budgeting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Host memory as measured by `detect_system_memory`. Surfaced in the `health` response because it sizes the Adaptive
/// profile's import budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemMemoryInfo {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Memory available to this process for import budgets, in bytes. Read
    /// from `MemAvailable` on Linux; on macOS, where no such figure is read,
    /// it is half of the total (an estimate).
    pub available_bytes: u64,
}

/// Host memory could not be measured by any supported source.
#[derive(Debug, thiserror::Error)]
#[error("system memory could not be detected: {causes}")]
pub struct MemoryDetectError {
    /// One entry per source tried, with why it failed.
    causes: String,
}

/// Process-wide cache of detected host memory so repeated `effective_limits`
/// and `health` calls do not re-read `/proc/meminfo` or spawn `sysctl`.
static CACHED_SYSTEM_MEMORY: OnceLock<SystemMemoryInfo> = OnceLock::new();

/// Detects host physical and available memory, caching a successful reading
/// for the life of the process.
///
/// Tries `/proc/meminfo` (Linux) and `sysctl hw.memsize` (macOS) in turn.
/// When neither yields a value this fails closed with every cause (and caches
/// nothing): the Adaptive profile is never sized off a machine that was not
/// measured, and the caller (startup, `health`) reports the memory as
/// undetected. The one estimate is the macOS available figure (half of the
/// measured total). The `/proc` parse is used rather than `sysconf`/`statfs`
/// because the crate has no `libc` dependency.
///
/// # Errors
///
/// Returns [`MemoryDetectError`] when no source yields a value.
pub fn detect_system_memory() -> Result<SystemMemoryInfo, MemoryDetectError> {
    if let Some(&cached) = CACHED_SYSTEM_MEMORY.get() {
        return Ok(cached);
    }
    let detected = detect_from(std::fs::read_to_string("/proc/meminfo"))?;
    Ok(*CACHED_SYSTEM_MEMORY.get_or_init(|| detected))
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
    let available_bytes = kb_to_bytes(avail_kb.ok_or("no MemAvailable line")?)?;
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
#[path = "system_ram_tests.rs"]
mod tests;
