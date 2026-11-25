//! Bounded child-process runner for self-update helpers (`curl`, target
//! `--version`), capping both stdout and stderr in Rust regardless of child
//! behavior or chunked transfer encoding.

use super::UpdateError;
use super::update_fetch::ASSET;
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Largest stderr output buffered from a child process, in bytes.
const STDERR_MAX_BYTES: u64 = 64 * 1024;

/// Reads up to `limit` bytes from `pipe`, returning whether the stream
/// produced at least one byte beyond `limit`.
fn read_pipe_capped(
    pipe: impl std::io::Read,
    limit: u64,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut buf = Vec::new();
    // KEEP saturating: reading one byte past the cap detects overflow without wrapping.
    let cap = limit.saturating_add(1);
    pipe.take(cap).read_to_end(&mut buf)?;
    let exceeded = match u64::try_from(buf.len()) {
        Ok(len) => len > limit,
        Err(_) => true,
    };
    if exceeded && let Ok(max_len) = usize::try_from(limit) {
        buf.truncate(max_len);
    }
    Ok((buf, exceeded))
}

/// Runs `command` with bounded stdout (when `stdout_limit` is `Some`) and
/// bounded stderr (`STDERR_MAX_BYTES`), preventing unbounded buffering.
pub(super) fn run_bounded(
    command: &mut Command,
    tool: &'static str,
    stdout_limit: Option<u64>,
) -> Result<Vec<u8>, UpdateError> {
    let stdout_cfg = if stdout_limit.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout_cfg)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| UpdateError::ToolSpawn { tool, source })?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (stdout_res, stderr_res) = std::thread::scope(|scope| {
        let stderr_task = scope.spawn(move || match stderr_pipe {
            Some(pipe) => read_pipe_capped(pipe, STDERR_MAX_BYTES),
            None => Ok((Vec::new(), false)),
        });
        let stdout_res = match (stdout_pipe, stdout_limit) {
            (Some(pipe), Some(limit)) => read_pipe_capped(pipe, limit),
            _ => Ok((Vec::new(), false)),
        };
        if matches!(stdout_res, Ok((_, true)))
            && let Err(err) = child.kill()
            && err.kind() != std::io::ErrorKind::InvalidInput
        {
            tracing::warn!(tool, error = %err, "could not terminate oversized child");
        }
        let stderr_res = match stderr_task.join() {
            Ok(res) => res,
            Err(_) => Err(std::io::Error::other("stderr reader thread panicked")),
        };
        (stdout_res, stderr_res)
    });
    let status = child
        .wait()
        .map_err(|err| UpdateError::Network(err.to_string()))?;
    let (stdout, stdout_exceeded) =
        stdout_res.map_err(|err| UpdateError::Network(err.to_string()))?;
    let (stderr, _) = stderr_res.map_err(|err| UpdateError::Network(err.to_string()))?;
    if let Some(limit) = stdout_limit
        && stdout_exceeded
    {
        return Err(UpdateError::Network(format!(
            "response exceeded {limit} bytes"
        )));
    }
    if !status.success() {
        return Err(UpdateError::Network(
            String::from_utf8_lossy(&stderr).trim().to_string(),
        ));
    }
    Ok(stdout)
}

/// Whether two binary paths refer to the same file, comparing canonical paths
/// when both resolve and falling back to raw path equality otherwise.
fn same_binary_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Extracts `<ver>` from `slugaudit <ver>` printed by `--version`.
pub(super) fn parse_binary_version(stdout: &str) -> Option<String> {
    let mut tokens = stdout.split_whitespace();
    let name = tokens.next()?;
    let version = tokens.next()?;
    (name == "slugaudit" && tokens.next().is_none()).then(|| version.to_owned())
}

/// Returns the version of `target`: the compiled-in version when `target` is
/// the running binary, otherwise the version reported by `target --version`.
pub(super) fn installed_version(running: &Path, target: &Path) -> String {
    if same_binary_path(running, target) {
        return env!("CARGO_PKG_VERSION").to_owned();
    }
    let mut cmd = Command::new(target);
    cmd.arg("--version");
    match run_bounded(&mut cmd, "slugaudit", Some(1024)) {
        Ok(stdout) => {
            let text = String::from_utf8_lossy(&stdout);
            if let Some(version) = parse_binary_version(&text) {
                return version;
            }
            tracing::warn!(
                path = %target.display(),
                output = text.trim(),
                "unexpected --version output from target binary",
            );
        }
        Err(error) => {
            tracing::warn!(
                path = %target.display(),
                error = %error,
                "could not query target binary version",
            );
        }
    }
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Parses the 64-hex-digit SHA-256 entry for `ASSET` from `SHA256SUMS` text,
/// accepting optional binary-mode `*` and `./` filename prefixes.
pub(super) fn parse_expected_digest(sums: &str) -> Result<String, UpdateError> {
    sums.lines()
        .find_map(|line| {
            let mut tokens = line.split_whitespace();
            let hash = tokens.next()?;
            let raw_name = tokens.next()?;
            if tokens.next().is_some() {
                return None;
            }
            let name = match raw_name.strip_prefix('*') {
                Some(stripped) => stripped,
                None => raw_name,
            };
            let name = match name.strip_prefix("./") {
                Some(stripped) => stripped,
                None => name,
            };
            let valid_hex = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
            (name == ASSET && valid_hex).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| {
            UpdateError::Checksum(format!(
                "SHA256SUMS has no entry for {ASSET}; refusing to update"
            ))
        })
}
