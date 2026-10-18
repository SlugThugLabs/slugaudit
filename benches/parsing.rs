#![forbid(unsafe_code)]

//! Parsing benchmarks: cold parser load (the first grammar load in this
//! process, including the language pack's on-demand grammar work) and warm
//! per-file parse + evidence extraction through SlugAudit's normalization
//! boundary (`evidence::extract`) for each benchmarked language.
//!
//! The cold-load value is captured once, before any criterion benchmark
//! runs, and printed to stderr so it lands in the same output as the
//! steady-state numbers and can be recorded in the committed baseline
//! `.planning/archive/perf_baseline.json` that `check_performance` consumes.
//! The steady-state benches therefore measure
//! warm-cache parse+extract, which is the cost a running server pays per
//! changed file during sync.

#[path = "common/samples.rs"]
mod samples;

use criterion::{Criterion, criterion_group, criterion_main};
use slugaudit_mcp_rust::evidence::extract;
use std::hint::black_box;
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
            let (items, _depth_capped) =
                extract(black_box(language), black_box(sample)).expect("extract");
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
        // A missing grammar would make its extract bench meaningless, so a
        // load failure aborts the run and carries the grammar error.
        match get_parser(language) {
            Ok(parser) => drop(parser),
            Err(error) => panic!("{language} parser failed to load: {error}"),
        }
    }

    let mut group = c.benchmark_group("parsing");
    group.sample_size(15);
    bench_extract(&mut group, "extract_rust", "rust", samples::RUST_SAMPLE);
    bench_extract(
        &mut group,
        "extract_python",
        "python",
        samples::PYTHON_SAMPLE,
    );
    bench_extract(
        &mut group,
        "extract_javascript",
        "javascript",
        samples::JS_SAMPLE,
    );
    bench_extract(
        &mut group,
        "extract_typescript",
        "typescript",
        samples::TS_SAMPLE,
    );
    group.finish();
}

criterion_group!(benches, bench_parsing);
criterion_main!(benches);
