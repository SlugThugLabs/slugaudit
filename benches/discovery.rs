//! Discovery benchmarks: the ignore-aware filesystem walk with binary
//! sniffing that every sync begins with, and the walk + per-file BLAKE3
//! hash pipeline (`discover` then `hash_file` for every indexed file) that
//! feeds the manifest comparison.

mod common;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slugaudit_mcp_rust::sync::{FileKind, discover, hash_file};

fn walk(root: &std::path::Path) -> usize {
    let (files, skipped) = discover(root).expect("discover");
    black_box(files.len() + skipped.len())
}

fn walk_and_hash(root: &std::path::Path) -> u64 {
    let (files, _skipped) = discover(root).expect("discover");
    let mut digest_chars = 0_u64;
    for file in &files {
        if file.kind == FileKind::Indexed {
            let identity = hash_file(&file.relative_path, &file.absolute_path).expect("hash");
            digest_chars += identity.content_hash.len() as u64;
        }
    }
    black_box(digest_chars)
}

fn bench_discovery(c: &mut Criterion) {
    let small = tempfile::tempdir().expect("temp dir");
    let large = tempfile::tempdir().expect("temp dir");
    let small_stats = common::generate_fixture(small.path(), common::SMALL);
    let large_stats = common::generate_fixture(large.path(), common::LARGE);
    eprintln!(
        "fixture_stats: small={{files={}, bytes={}}} large={{files={}, bytes={}}}",
        small_stats.file_count,
        small_stats.total_bytes,
        large_stats.file_count,
        large_stats.total_bytes,
    );

    let mut group = c.benchmark_group("discovery");
    group.sample_size(30);

    group.bench_function("walk_small", |b| b.iter(|| walk(small.path())));
    group.bench_function("walk_large", |b| b.iter(|| walk(large.path())));
    group.bench_function("walk_and_hash_small", |b| {
        b.iter(|| walk_and_hash(small.path()))
    });
    group.bench_function("walk_and_hash_large", |b| {
        b.iter(|| walk_and_hash(large.path()))
    });

    group.finish();
}

criterion_group!(benches, bench_discovery);
criterion_main!(benches);
