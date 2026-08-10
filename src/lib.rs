#![forbid(unsafe_code)]
// Stdout is the MCP JSON-RPC transport in serve mode. Any `println!` or
// `print!` that reaches it mid-serve corrupts the stream and fails every
// in-flight call. `clippy::print_stdout` is denied at the crate root so a
// stray `println!` in serve-reachable code is a compile error, not a
// runtime surprise. `cli.rs` (and the binary's `main.rs`) are allowed
// below because their stdout writes happen before serve mode, as the
// user-facing CLI interface.
#![deny(clippy::print_stdout)]

pub mod cli;
pub mod connect;
pub mod evidence;
pub mod graph;
pub mod install;
pub mod model;
pub mod parse;
pub mod project;
pub mod server;
pub mod store;
pub mod sync;
pub mod tools;
pub mod util;
