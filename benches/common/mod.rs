//! Shared fixture generation and helpers for the benchmark suite.
//!
//! Every item here is used by every bench binary that includes this module
//! (`discovery`, `search`, `memory`); the parsing samples live in
//! `samples.rs` and the fixture sizes only some benches need are declared by
//! those benches, so no dead-code waiver is required.
//!
//! All content is generated deterministically from the file count, so the
//! same fixture is reproducible across runs and machines (Task 9.2:
//! "benchmarks run reproducibly on the same fixture"). The tree covers five
//! source families (Rust, Python, JavaScript, TypeScript, plus Markdown and
//! JSON config), real project-local imports that exercise dependency-edge
//! resolution during publish, external imports, two intentionally malformed
//! source files (so parser diagnostics are part of the workload), and the
//! `.planning/slugaudit/` activation marker every real project has.

use std::fs;
use std::path::Path;

/// Large fixture size: 200 files (the primary baseline workload).
pub(crate) const LARGE: usize = 200;

/// Counts describing the generated tree, so benchmarks can report on the
/// fixture without re-deriving the layout rules.
pub(crate) struct FixtureStats {
    file_count: usize,
    total_bytes: u64,
}

impl FixtureStats {
    /// Print the fixture's size to stderr under `label`.
    pub(crate) fn log(&self, label: &str) {
        eprintln!(
            "fixture_stats: {label}={{files={}, bytes={}}}",
            self.file_count, self.total_bytes
        );
    }
}

/// Deterministic content generator (splitmix64) — no external RNG, no
/// process-global state, so fixture content never varies between runs.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// Project-local import target for a per-language index `k`: a fixed
/// permutation so every file imports *some* sibling (exercising resolution
/// and edge building) without forming a degenerate all-to-one graph.
fn sibling(k: usize, count: usize) -> usize {
    (k * 7 + 3) % count
}

/// Generates the full fixture tree under `root`. Returns per-fixture stats.
///
/// Layout cycle (i % 10): 0=markdown, 1/2/8=Rust, 3/4/9=Python,
/// 5=JavaScript, 6=TypeScript, 7=JSON config. With `file_count` divisible
/// by 10 the per-language counts are exact.
#[must_use]
pub(crate) fn generate_fixture(root: &Path, file_count: usize) -> FixtureStats {
    let mut mix = Mix(0xC0FF_EE00_DEAD_BEEF);
    let mut total_bytes = 0_u64;

    fs::create_dir_all(root.join(".planning").join("slugaudit")).expect("activation dir");
    fs::create_dir_all(root.join("src/rust")).expect("rust dir");
    fs::create_dir_all(root.join("src/python")).expect("python dir");
    fs::create_dir_all(root.join("src/js")).expect("js dir");
    fs::create_dir_all(root.join("src/ts")).expect("ts dir");
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::create_dir_all(root.join("config")).expect("config dir");

    let rust_count = file_count / 10 * 3;
    let python_count = file_count / 10 * 3;
    let js_count = file_count / 10;
    let ts_count = file_count / 10;

    let mut rust_index = 0_usize;
    let mut python_index = 0_usize;
    let mut js_index = 0_usize;
    let mut ts_index = 0_usize;

    let mut write = |relative: &str, content: &str| {
        let path = root.join(relative);
        fs::write(&path, content).expect("write fixture file");
        total_bytes += content.len() as u64;
    };

    write(".gitignore", "target/\nnode_modules/\n");

    for i in 0..file_count {
        match i % 10 {
            0 => write(
                &format!("docs/doc_{i}.md"),
                &format!(
                    "# Doc {i}\n\nReference documentation mentioning shared_helper and needle_{i}.\n\
                     Some more prose to give the parser something to read.\n"
                ),
            ),
            1 | 2 | 8 => {
                let k = rust_index;
                rust_index += 1;
                write(
                    &format!("src/rust/mod_{k}.rs"),
                    &rust_source(k, sibling(k, rust_count), mix.below(100) as u64),
                );
            }
            3 | 4 | 9 => {
                let k = python_index;
                python_index += 1;
                write(
                    &format!("src/python/mod_{k}.py"),
                    &python_source(k, sibling(k, python_count), mix.below(100) as u64),
                );
            }
            5 => {
                let k = js_index;
                js_index += 1;
                write(
                    &format!("src/js/mod_{k}.js"),
                    &js_source(k, sibling(k, js_count), mix.below(100) as u64),
                );
            }
            6 => {
                let k = ts_index;
                ts_index += 1;
                write(
                    &format!("src/ts/mod_{k}.ts"),
                    &ts_source(k, sibling(k, ts_count), mix.below(100) as u64),
                );
            }
            _ => write(
                &format!("config/config_{i}.json"),
                &format!(
                    "{{\n  \"name\": \"config_{i}\",\n  \"enabled\": true,\n  \
                     \"retries\": 3,\n  \"needle\": \"needle_{i}\",\n  \
                     \"helper\": \"shared_helper\"\n}}\n"
                ),
            ),
        }
    }

    // Two intentionally malformed sources so parser diagnostics are part of
    // every sync's workload, matching real trees.
    write(
        "src/python/broken.py",
        "def broken(:\n    return 1\n\ndef fine():\n    return 2\n",
    );
    write(
        "src/js/broken.js",
        "function broken( {\n  return 1;\n}\n\nexport function fine() {\n  return 2;\n}\n",
    );

    FixtureStats {
        file_count,
        total_bytes,
    }
}

fn rust_source(k: usize, neighbor: usize, offset: u64) -> String {
    format!(
        r#"// Generated benchmark fixture file {k}
use crate::mod_{neighbor};
use std::collections::HashMap;

pub struct Widget {{
    pub name: String,
    pub value: u64,
}}

impl Widget {{
    pub fn new(name: &str, value: u64) -> Self {{
        Self {{ name: name.to_owned(), value }}
    }}

    pub fn scaled(&self, factor: u64) -> u64 {{
        self.value.saturating_mul(factor + {offset})
    }}
}}

pub fn needle_{k}() -> u64 {{
    42 + {offset}
}}

pub fn shared_helper(value: u64) -> u64 {{
    let mut map = HashMap::new();
    map.insert("key", value);
    map.get("key").copied().unwrap_or(0)
}}

pub fn use_neighbor() -> u64 {{
    mod_{neighbor}::needle_{neighbor}()
}}

pub fn classify(value: u64) -> &'static str {{
    match value {{
        0 => "zero",
        1..=9 => "small",
        10..=99 => "medium",
        _ => "large",
    }}
}}
"#
    )
}

fn python_source(k: usize, neighbor: usize, offset: u64) -> String {
    format!(
        r#"# Generated benchmark fixture file {k}
import os

from .mod_{neighbor} import needle_{neighbor}


class Widget:
    """A small widget with a name and a value."""

    def __init__(self, name: str, value: int = 0) -> None:
        self.name = name
        self.value = value

    def scaled(self, factor: int) -> int:
        return self.value * factor + {offset}


def needle_{k}() -> int:
    return 42 + {offset}


def shared_helper(value: int) -> int:
    return value * 2


def use_neighbor() -> int:
    return needle_{neighbor}()
"#
    )
}

fn js_source(k: usize, neighbor: usize, offset: u64) -> String {
    format!(
        r#"// Generated benchmark fixture file {k}
import {{ needle_{neighbor} }} from './mod_{neighbor}';
import fs from 'fs';

export class Widget {{
  constructor(name, value = 0) {{
    this.name = name;
    this.value = value;
  }}

  scaled(factor) {{
    return this.value * factor + {offset};
  }}
}}

export function needle_{k}() {{
  return 42 + {offset};
}}

export function shared_helper(value) {{
  return value * 2;
}}

export function use_neighbor() {{
  return needle_{neighbor}();
}}
"#
    )
}

fn ts_source(k: usize, neighbor: usize, offset: u64) -> String {
    format!(
        r#"// Generated benchmark fixture file {k}
import {{ needle_{neighbor} }} from './mod_{neighbor}';

export interface Widget {{
  name: string;
  value: number;
}}

export class WidgetImpl implements Widget {{
  constructor(public name: string, public value: number = 0) {{}}

  scaled(factor: number): number {{
    return this.value * factor + {offset};
  }}
}}

export function needle_{k}(): number {{
  return 42 + {offset};
}}

export function shared_helper(value: number): number {{
  return value * 2;
}}

export function use_neighbor(): number {{
  return needle_{neighbor}();
}}
"#
    )
}
