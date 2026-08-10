//! Parsing benchmarks: cold parser load (the first grammar load in this
//! process, including the language pack's on-demand grammar work) and warm
//! per-file parse + evidence extraction through SlugAudit's normalization
//! boundary (`evidence::extract`) for each benchmarked language.
//!
//! The cold-load value is captured once, before any criterion benchmark
//! runs, and printed to stderr so it lands in the same output as the
//! steady-state numbers and can be recorded in
//! `.planning/PERFORMANCE.md`. The steady-state benches therefore measure
//! warm-cache parse+extract, which is the cost a running server pays per
//! changed file during sync.

mod common;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slugaudit_mcp_rust::evidence::extract;
use std::time::Instant;
use tree_sitter_language_pack::get_parser;

fn bench_extract<'a>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    language: &'a str,
    sample: &'a str,
) {
    group.bench_function(name, |b| {
        b.iter(|| {
            let items = extract(black_box(language), black_box(sample)).expect("extract");
            black_box(items.len());
        });
    });
}

fn bench_parsing(c: &mut Criterion) {
    let cold_start = Instant::now();
    let parser = match get_parser("rust") {
        Ok(parser) => parser,
        Err(error) => {
            // Offline / missing-grammar environment: report and skip rather
            // than failing the whole bench run. Sync benchmarks degrade
            // gracefully; this one cannot.
            eprintln!(
                "parser unavailable (offline or missing grammar): {error}; skipping parsing benches"
            );
            return;
        }
    };
    eprintln!("parser_cold_load_rust: {:?}", cold_start.elapsed());
    drop(parser);

    // Force-load the remaining grammars before any benchmark runs so every
    // extract bench below measures genuinely warm parse + extraction (the
    // first sample of each bench would otherwise include that grammar's
    // one-time load, which is exactly what the cold-load line captures for
    // Rust).
    for language in ["python", "javascript", "typescript"] {
        let loaded = get_parser(language);
        assert!(
            loaded.is_ok(),
            "{language} parser loads; a missing grammar would make its extract bench meaningless"
        );
        drop(loaded);
    }

    let mut group = c.benchmark_group("parsing");
    bench_extract(&mut group, "extract_rust", "rust", common::RUST_SAMPLE);
    bench_extract(
        &mut group,
        "extract_python",
        "python",
        common::PYTHON_SAMPLE,
    );
    bench_extract(
        &mut group,
        "extract_javascript",
        "javascript",
        common::JS_SAMPLE,
    );
    bench_extract(
        &mut group,
        "extract_typescript",
        "typescript",
        common::TS_SAMPLE,
    );
    group.finish();
}

criterion_group!(benches, bench_parsing);
criterion_main!(benches);
