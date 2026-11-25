use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn hash_chunk_bytes_is_64_kib() -> TestResult {
    ensure_eq(&HASH_CHUNK_BYTES, &65536, "HASH_CHUNK_BYTES == 65536")?;
    Ok(())
}

struct MockInterruptedReader<'a> {
    interrupted_once: bool,
    data: &'a [u8],
}

impl std::io::Read for MockInterruptedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.interrupted_once {
            self.interrupted_once = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "interrupted",
            ));
        }
        let to_read = std::cmp::min(buf.len(), self.data.len());
        let (Some(target), Some((chunk, rest))) =
            (buf.get_mut(..to_read), self.data.split_at_checked(to_read))
        else {
            return Err(std::io::Error::other("read split out of range"));
        };
        target.copy_from_slice(chunk);
        self.data = rest;
        Ok(to_read)
    }
}

#[test]
fn hash_reader_retries_on_interrupted() -> TestResult {
    let mut reader = MockInterruptedReader {
        interrupted_once: false,
        data: b"hello world",
    };
    let dummy_path = std::path::Path::new("dummy.rs");
    let result = hash_reader(&mut reader, "dummy.rs", dummy_path, 1024)
        .check("interrupted read must be retried")?;
    ensure_eq(
        &result.content_hash,
        &crate::sync::hash::hash_bytes("dummy.rs", b"hello world").content_hash,
        "result.content_hash == crate::sync::hash::hash_bytes(\"dummy.rs\", b\"hello world\").content_hash",
    )?;
    Ok(())
}

struct MockNonInterruptedErrorReader {
    called: bool,
    retried: bool,
}

impl std::io::Read for MockNonInterruptedErrorReader {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        if self.called {
            self.retried = true;
        }
        self.called = true;
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ))
    }
}

#[test]
fn hash_reader_fails_on_non_interrupted_error() -> TestResult {
    let mut reader = MockNonInterruptedErrorReader {
        called: false,
        retried: false,
    };
    let dummy_path = std::path::Path::new("dummy.rs");
    let result = hash_reader(&mut reader, "dummy.rs", dummy_path, 1024);
    ensure(
        matches!(result, Err(HashError::Read { .. })),
        "matches!(result, Err(HashError::Read { .. }))",
    )?;
    ensure(!reader.retried, "non-interrupted error must not be retried")?;
    Ok(())
}

/// A reader that claims to have filled more bytes than the buffer holds.
struct OverreportingReader;

impl Read for OverreportingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        buf.len()
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("buffer length is usize::MAX"))
    }
}

#[test]
fn hash_reader_rejects_a_read_count_past_the_buffer() -> TestResult {
    let dummy_path = std::path::Path::new("dummy.rs");
    let result = hash_reader(OverreportingReader, "dummy.rs", dummy_path, u64::MAX);
    let Err(HashError::Read { path, source }) = result else {
        return Err("an over-reported read must fail with HashError::Read".to_owned())
            .check("unexpected outcome");
    };
    ensure_eq(&path, &dummy_path, "path == dummy_path")?;
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::InvalidData,
        "source.kind() == std::io::ErrorKind::InvalidData",
    )?;
    ensure_eq(
        &source.to_string(),
        &format!(
            "reader reported {} bytes for a {}-byte buffer",
            HASH_CHUNK_BYTES + 1,
            HASH_CHUNK_BYTES
        ),
        "source.to_string() == format!( \"reader reported {} bytes for a {}-byte buffer\", HASH_CHUNK_BYTES + 1, HASH_CHUNK_BYTES )",
    )?;
    Ok(())
}

#[test]
fn hash_file_refuses_non_regular_file_with_invalid_input() -> TestResult {
    let dir = tempfile::tempdir().check("tempdir")?;
    let Err(HashError::Read { source, .. }) = hash_file("dir", dir.path(), 1024) else {
        return Err("directory must fail with HashError::Read".to_owned())
            .check("unexpected outcome");
    };
    ensure_eq(
        &source.kind(),
        &std::io::ErrorKind::InvalidInput,
        "non-regular file is rejected with InvalidInput",
    )?;
    Ok(())
}
