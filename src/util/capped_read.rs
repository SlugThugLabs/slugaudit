//! Bounded reads of foreign text files.
//!
//! The CLI edits other tools' configs and the user's shell rc; none of them
//! has a prior that it is small, so every read is stat-checked and the read
//! itself is length-bounded. A file that grows inside the stat/read window is
//! reported, never truncated.

use std::io::Read as _;

/// Cap for foreign text files the CLI reads to edit them: other tools'
/// configs and the user's shell rc. None of them has a prior that they are
/// small, so reads are bounded like everything else in this crate.
pub(crate) const MAX_FOREIGN_TEXT_BYTES: u64 = 1024 * 1024;

/// Why a capped file read failed.
#[derive(Debug)]
pub(crate) enum CappedReadError {
    /// The file could not be inspected or read.
    Io(std::io::Error),
    /// The file exceeds the caller's cap. `observed` is the stat size when
    /// that was already over the cap, or the bounded-read length when the
    /// file grew in the stat/read window; it is never a truncation.
    TooLarge { limit: u64, observed: u64 },
}

impl std::fmt::Display for CappedReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::TooLarge { limit, observed } => {
                write!(
                    formatter,
                    "file is {observed} bytes, over the {limit}-byte cap"
                )
            }
        }
    }
}

impl std::error::Error for CappedReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::TooLarge { .. } => None,
        }
    }
}

/// Platform `O_NONBLOCK` flag for opening paths without blocking on FIFOs.
#[cfg(any(target_os = "linux", target_os = "android"))]
const O_NONBLOCK: i32 = 0o4000;
/// Platform `O_NONBLOCK` flag for opening paths without blocking on FIFOs.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
))]
const O_NONBLOCK: i32 = 0x0004;

/// Opens `path` as a regular file without blocking on FIFOs or other special
/// files. On Unix targets with a known `O_NONBLOCK` value, opens with
/// `O_NONBLOCK` first and verifies `is_file()` on the opened descriptor so a
/// FIFO, directory, or device never blocks `open` or enters a file read.
pub(crate) fn open_regular_file_nonblocking(
    path: &std::path::Path,
) -> std::io::Result<(std::fs::File, std::fs::Metadata)> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
    ))]
    let file = {
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)?
    };
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
    )))]
    let file = {
        let pre = std::fs::metadata(path)?;
        if !pre.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", path.display()),
            ));
        }
        std::fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok((file, metadata))
}

/// Reads a file's bytes, refusing to load more than `max_bytes`.
///
/// Only a regular file is read: opening a FIFO without `O_NONBLOCK` blocks
/// until a writer appears, so [`open_regular_file_nonblocking`] opens
/// non-blocking on Unix and verifies `is_file()` on the opened descriptor,
/// returning `InvalidInput` for a FIFO, directory, or device. The opened
/// descriptor's metadata also rejects an oversized file before allocating; the
/// read itself is bounded by `take(max_bytes + 1)`, so a file that grows past
/// the cap between the stat and the read cannot allocate an unbounded buffer.
/// The observed length is reported, never truncated.
pub(crate) fn read_file_capped_bytes(
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<Vec<u8>, CappedReadError> {
    let (file, metadata) = open_regular_file_nonblocking(path).map_err(CappedReadError::Io)?;
    if metadata.len() > max_bytes {
        return Err(CappedReadError::TooLarge {
            limit: max_bytes,
            observed: metadata.len(),
        });
    }
    let mut bytes = match usize::try_from(metadata.len().min(64 * 1024)) {
        Ok(cap) => Vec::with_capacity(cap),
        Err(_) => Vec::new(),
    };
    // KEEP saturating: at a cap of u64::MAX no read can exceed it, so reading up to u64::MAX bytes loses nothing
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(CappedReadError::Io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(CappedReadError::TooLarge {
            limit: max_bytes,
            observed: bytes.len() as u64,
        });
    }
    Ok(bytes)
}

/// Reads a UTF-8 file whose size must not exceed `max_bytes`.
///
/// Delegates the bound to [`read_file_capped_bytes`] so both the stat and
/// the read are capped; a file that grows in the window is reported, not
/// loaded. The returned error keeps `NotFound` and other I/O kinds intact
/// for callers that branch on them.
pub(crate) fn read_file_capped(path: &std::path::Path, max_bytes: u64) -> std::io::Result<String> {
    let bytes = read_file_capped_bytes(path, max_bytes).map_err(|error| match error {
        CappedReadError::Io(source) => source,
        too_large @ CappedReadError::TooLarge { .. } => {
            std::io::Error::new(std::io::ErrorKind::InvalidData, too_large.to_string())
        }
    })?;
    String::from_utf8(bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("file is not valid UTF-8: {error}"),
        )
    })
}
