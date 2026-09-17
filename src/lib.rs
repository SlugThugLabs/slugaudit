#![forbid(unsafe_code)]
// Stdout is the MCP JSON-RPC transport in serve mode. Any `println!` or
// `print!` that reaches it mid-serve corrupts the stream and fails every
// in-flight call. `clippy::print_stdout` is denied at the crate root so a
// stray `println!` in serve-reachable code is a compile error, not a
// runtime surprise. `cli.rs` (and the binary's `main.rs`) are allowed
// below because their stdout writes happen before serve mode, as the
// user-facing CLI interface.
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
pub mod connect;
pub mod connect_agents;
pub mod connect_exec;
pub mod evidence;
pub mod graph;
pub mod ignore_rules;
pub mod install;
pub mod menu;
pub(crate) mod menu_render;
pub mod model;
pub mod parse;
pub mod progress;
pub mod project;
pub mod server;
mod server_runner;
pub mod store;
pub mod sync;
pub mod tools;
pub mod update;
pub mod util;
pub mod watch;
