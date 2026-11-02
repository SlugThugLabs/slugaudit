#![forbid(unsafe_code)]

//! Discovery benchmarks: the ignore-aware filesystem walk with binary
//! sniffing that every sync begins with, and the walk + per-file BLAKE3
//! hash pipeline (`discover` then `hash_file` for every indexed file) that
//! feeds the manifest comparison.

mod common;

use common::failure::{FirstFailure, exit_code};

/// Small fixture size: 40 files (the cheap contrast to `common::LARGE`).
const SMALL: usize = 40;

/// The generated fixtures are far below this cap; it keeps the benchmark on
/// the same bounded hashing API production uses instead of an unbounded call.
const HASH_CAP_BYTES: u64 = 64 * 1024 * 1024;

use criterion::Criterion;
use slugaudit_mcp_rust::sync::{FileKind, discover, hash_file};
use slugaudit_test_check::{Check, TestResult};
use std::hint::black_box;
use std::process::ExitCode;

fn walk(root: &std::path::Path) -> TestResult<usize> {
    let (files, skipped) = discover(root).check("discover files")?;
    Ok(black_box(files.len() + skipped.len()))
}

fn walk_and_hash(root: &std::path::Path) -> TestResult<u64> {
    let (files, _skipped) = discover(root).check("discover files")?;
    let mut digest_chars = 0_u64;
    for file in &files {
        if file.kind == FileKind::Indexed {
            let identity = hash_file(&file.relative_path, &file.absolute_path, HASH_CAP_BYTES)
                .check("hash an indexed file")?;
            digest_chars += identity.content_hash.len() as u64;
        }
    }
    Ok(black_box(digest_chars))
}

fn bench_discovery(c: &mut Criterion) -> TestResult {
    let small = tempfile::tempdir().check("create small fixture dir")?;
    let large = tempfile::tempdir().check("create large fixture dir")?;
    common::generate_fixture(small.path(), SMALL)?.log("small");
    common::generate_fixture(large.path(), common::LARGE)?.log("large");
    let failure = FirstFailure::new();

    let mut group = c.benchmark_group("discovery");
    group.sample_size(15);

    group.bench_function("walk_small", |b| {
        b.iter(|| failure.record(walk(small.path())));
    });
    group.bench_function("walk_large", |b| {
        b.iter(|| failure.record(walk(large.path())));
    });
    group.bench_function("walk_and_hash_small", |b| {
        b.iter(|| failure.record(walk_and_hash(small.path())));
    });
    group.bench_function("walk_and_hash_large", |b| {
        b.iter(|| failure.record(walk_and_hash(large.path())));
    });

    group.finish();
    failure.finish()
}

fn main() -> ExitCode {
    let mut criterion = Criterion::default().configure_from_args();
    let result = bench_discovery(&mut criterion);
    criterion.final_summary();
    exit_code(result)
}
