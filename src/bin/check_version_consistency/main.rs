#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]
// Same production panic ban as the library root; see `lib.rs` for why
// `expect` is denied alongside `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

//! Version-consistency gate.
//!
//! The released version is hand-duplicated: `Cargo.toml`'s `[package]
//! version`, `lhm.plugin.json`'s `version`, and the newest released
//! `CHANGELOG.md` heading are three copies of the same string, and
//! `src/parse/mod.rs`'s `PACK_VERSION` is a fourth copy of the
//! `tree-sitter-language-pack` pin. Nothing in the build ties those copies
//! together, so a bump that misses one ships a lie. This gate ties them.
//!
//! Checks ([`manifests`]):
//!
//! 1. `Cargo.toml` version == `lhm.plugin.json` version.
//! 2. newest `CHANGELOG.md` release heading == the `Cargo.toml` version.
//! 3. `parse::PACK_VERSION` == the exact (`=`) `tree-sitter-language-pack`
//!    pin in `Cargo.toml`. This was a unit test
//!    (`src/parse/mod.rs::pack_version_matches_the_cargo_toml_pin`) and now
//!    lives here, so the check reads the manifest from a chosen root rather
//!    than only the crate's own compile-time `include_str!`.
//!
//! Pure-Rust: no Python, no shell. `Cargo.toml` is scanned textually (no
//! TOML parser is a dependency); `lhm.plugin.json` is parsed with
//! `serde_json`, already a dependency.

mod manifests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut root = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("version-consistency: cannot determine project root: {error}");
            return ExitCode::from(2);
        }
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!(
                    "Checks that Cargo.toml, lhm.plugin.json, CHANGELOG.md and parse::PACK_VERSION \
                     all agree on the version."
                );
                return ExitCode::SUCCESS;
            }
            "--root" => {
                let Some(value) = args.next() else {
                    eprintln!("--root requires a project directory");
                    return ExitCode::from(2);
                };
                root = PathBuf::from(value);
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let failures = match manifests::check(&root) {
        Ok(failures) => failures,
        Err(error) => {
            eprintln!("version-consistency: {error}");
            return ExitCode::from(2);
        }
    };

    for failure in &failures {
        println!("version-consistency: {failure}");
    }
    if !failures.is_empty() {
        println!("version-consistency: FAIL");
        return ExitCode::from(1);
    }
    println!("version-consistency: PASS");
    ExitCode::SUCCESS
}
