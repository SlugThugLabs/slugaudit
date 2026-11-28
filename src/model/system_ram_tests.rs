use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn parse_proc_meminfo_reads_valid_data() -> TestResult {
    let sample =
        "MemTotal:       16384000 kB\nMemFree:         8192000 kB\nMemAvailable:   12288000 kB\n";
    let info = parse_proc_meminfo(sample).check("parse succeeds")?;
    ensure_eq(
        &info.total_bytes,
        &(16_384_000 * 1024),
        "MemTotal kB converts to exact byte count",
    )?;
    ensure_eq(
        &info.available_bytes,
        &(12_288_000 * 1024),
        "MemAvailable kB converts to exact byte count",
    )?;
    Ok(())
}

#[test]
fn parse_proc_meminfo_refuses_a_missing_memavailable() -> TestResult {
    let sample = "MemTotal:       16384000 kB\nMemFree:         8192000 kB\n";
    let error = parse_proc_meminfo(sample).check_err("no MemAvailable line")?;
    ensure_eq(
        &error,
        &"no MemAvailable line",
        "missing MemAvailable produces specific error message",
    )?;
    Ok(())
}

#[test]
fn parse_proc_meminfo_handles_missing_memtotal() -> TestResult {
    let sample = "MemFree:         8192000 kB\nMemAvailable:   12288000 kB\n";
    ensure(
        parse_proc_meminfo(sample).is_err(),
        "missing MemTotal fails meminfo parsing",
    )?;
    ensure(
        parse_proc_meminfo("invalid content").is_err(),
        "unstructured content fails meminfo parsing",
    )?;
    Ok(())
}

#[test]
fn a_malformed_memtotal_line_names_the_line_in_its_error() -> TestResult {
    let error = parse_proc_meminfo("MemTotal:  lots kB\n").check_err("malformed value")?;

    ensure(error.contains("malformed value 'lots'"), &error)?;
    Ok(())
}

#[test]
fn a_memtotal_that_overflows_bytes_is_an_error_not_a_panic() -> TestResult {
    let sample = format!("MemTotal: {} kB\n", u64::MAX);

    let error = parse_proc_meminfo(&sample).check_err("overflow")?;

    ensure(error.contains("overflows"), &error)?;
    Ok(())
}

#[test]
fn a_sysctl_output_that_is_not_a_number_is_an_error_with_a_cause() -> TestResult {
    ensure_eq(
        &parse_sysctl_memsize(b"17179869184\n"),
        &Ok(17_179_869_184),
        "numeric sysctl output parses to total bytes",
    )?;
    ensure(
        parse_sysctl_memsize(b"n/a").is_err(),
        "non-numeric sysctl text fails parsing",
    )?;
    ensure(
        parse_sysctl_memsize(&[0xff]).is_err(),
        "non-UTF-8 sysctl output fails parsing",
    )?;
    Ok(())
}

#[test]
fn detect_returns_positive_cached_memory() -> TestResult {
    let first = detect_system_memory().check("this host reports its memory")?;
    let second = detect_system_memory().check("cached host memory returns cleanly")?;
    ensure(first.total_bytes > 0, "detected total memory is positive")?;
    ensure(
        first.available_bytes > 0,
        "detected available memory is positive",
    )?;
    ensure(
        first.available_bytes <= first.total_bytes,
        "available memory never exceeds total physical memory",
    )?;
    ensure_eq(
        &first,
        &second,
        "subsequent calls return the cached host memory reading",
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn fail_closed_undetectable_memory_is_an_error_naming_the_cause() -> TestResult {
    let unreadable = std::io::Error::new(std::io::ErrorKind::NotFound, "no meminfo here");

    let error = detect_from(Err(unreadable)).check_err("no source yields a value")?;

    ensure_eq(
        &error.to_string(),
        &"system memory could not be detected: /proc/meminfo: no meminfo here",
        "detection error names the failed memory source and its I/O cause",
    )?;
    Ok(())
}

#[test]
fn parse_windows_cim_kib_parses_total_and_free_kib() -> TestResult {
    let parsed = parse_windows_cim_kib(b"16777216 8388608\r\n").check("valid CIM KiB")?;
    ensure_eq(
        &parsed,
        &SystemMemoryInfo {
            total_bytes: 16_777_216 * 1024,
            available_bytes: 8_388_608 * 1024,
        },
        "total and available bytes converted from KiB",
    )?;
    let _ = parse_windows_cim_kib(b"0 0").check_err("zero total rejected")?;
    let _ = parse_windows_cim_kib(b"not-a-number").check_err("garbage rejected")?;
    Ok(())
}
