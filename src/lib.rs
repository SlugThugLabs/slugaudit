//! SlugAudit: a stdio MCP server that indexes a project into a disposable
//! SQLite database and answers evidence queries about it — parsed structure,
//! symbols, imports, diagnostics and session findings.
//!
//! The shipped artifact is the `slugaudit` binary (`src/main.rs`), which either
//! serves the MCP protocol on stdio or runs a CLI command (`install`, `update`,
//! `connect`, `menu`). These library modules are that binary's implementation;
//! the crate is `publish = false` and the single binary is the shipped
//! artifact.
//!
//! Runtime data lives in `<project>/.planning/slugaudit/project.db` and is
//! disposable: it is rebuilt from source when stale or corrupt. SlugAudit
//! reports evidence only; every judgment about it belongs to the calling agent.

#![forbid(unsafe_code)]
// Stdout is the MCP JSON-RPC transport in serve mode. Any `println!` or
// `print!` that reaches it mid-serve corrupts the stream and fails every
// in-flight call. `clippy::print_stdout` is denied at the crate root (and
// in `Cargo.toml` for every target) so a stray print macro is a compile
// error, not a runtime surprise. CLI output needs no file-level exception:
// it goes through `cli::out` / `cli::err`, which write and flush through
// `std::io::Write` and return the failure instead of panicking.
#![deny(clippy::print_stdout)]
// Production code must not panic on an error value. `unwrap` and `expect`
// are denied outside `cfg(test)`, where a panic is exactly the harness's
// failure signal and the conventional idiom. `expect` is included rather
// than exempted because the two differ only in whether the panic carries a
// reason: allowing `expect` while banning `unwrap` would push callers to
// the less informative form. Nothing in this crate needs either - fallible
// work returns `Result`, and the only genuinely fatal conditions (failed
// process startup) report the error and choose an exit code.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod cli;
pub(crate) mod clock;
pub mod connect;
pub mod connect_agents;
pub mod connect_cli;
pub mod connect_exec;
pub mod connect_guidance;
pub mod error_codes;
pub mod evidence;
pub mod graph;
pub mod ignore_rules;
pub mod install;
pub(crate) mod install_place;
pub mod menu;
pub(crate) mod menu_render;
pub mod model;
pub mod parse;
pub(crate) mod probe;
pub mod progress;
pub mod project;
pub(crate) mod prompt;
pub mod resources;
pub mod server;
mod server_runner;
pub mod store;
pub mod sync;
pub mod tools;
pub mod update;
pub mod util;
pub mod watch;
