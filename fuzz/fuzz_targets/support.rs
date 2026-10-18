//! One synced project, built once per fuzz process.
//!
//! `structure` and `query` both resolve the active project and re-verify its
//! index on every call (`tools::context::ensure_synced` → a stat sweep per
//! call), so building the project inside the fuzz loop would measure the sync
//! pipeline instead of the tool. The fixture is created on first use and
//! reused for the rest of the run; every iteration then fuzzes only the
//! request payload the tool actually owns.
//!
//! This file is not a fuzz target: it has no `[[bin]]` entry in
//! `fuzz/Cargo.toml`, so `cargo fuzz` never builds it standalone. Each target
//! pulls it in with `#[path = "support.rs"] mod support;`.

use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::sync::SourceSyncManager;
use slugaudit_mcp_rust::tools::ensure_synced;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Source files seeded into the fixture, one per grammar, so one query corpus
/// exercises several of the language pack's parsers (and `Query::new`'s
/// rejection of the same text for grammars that do not model it).
///
/// The bodies are deliberately mixed: nested functions and classes, call
/// sites, string literals, and non-ASCII text (CJK inside Python strings, an
/// emoji inside a Rust literal, an accented identifier in Go) so node spans
/// land on multi-byte boundaries and `model::span::char_column` — the offset
/// arithmetic that used to slice a string mid-codepoint — is exercised against
/// real content rather than ASCII.
pub const FILES: &[(&str, &str)] = &[
    (
        "src/lib.rs",
        r#"//! Fixture crate.
pub mod inner;

/// Greets `name` with a rocket.
pub fn greet(name: &str) -> String {
    format!("héllo, {name} — 🚀")
}

pub struct Widget {
    pub id: u32,
}

impl Widget {
    pub fn new(id: u32) -> Self {
        Self { id }
    }

    pub fn render(&self) -> String {
        greet(&format!("widget {}", self.id))
    }
}

pub fn sum(values: &[i32]) -> i32 {
    let mut total = 0;
    for value in values {
        total += *value;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_values() {
        assert_eq!(sum(&[1, 2, 3]), 6);
    }
}
"#,
    ),
    (
        "pkg/mod.py",
        r#"import json


class Greeter:
    """Greets a person."""

    def __init__(self, name):
        self.name = name

    def greet(self):
        return f"こんにちは {self.name}"


def load(path):
    with open(path) as handle:
        return json.load(handle)


def main():
    print(Greeter("world").greet())
"#,
    ),
    (
        "web/app.js",
        r#"export function add(a, b) {
  return a + b;
}

const greet = (name) => `hi ${name}`;

class App {
  constructor(name) {
    this.name = name;
  }

  render() {
    return greet(this.name);
  }
}

export default App;
"#,
    ),
    (
        "svc/main.go",
        r#"package main

import "fmt"

type Server struct {
	Addr string
}

func (s *Server) Run() error {
	fmt.Println(s.Addr)
	return nil
}

func main() {
	s := Server{Addr: "héllo"}
	if err := s.Run(); err != nil {
		panic(err)
	}
}
"#,
    ),
    (
        "src/index.ts",
        r#"interface User {
  id: number;
  name: string;
}

export function findUser(users: User[], id: number): User | undefined {
  return users.find((user) => user.id === id);
}

export class Registry {
  private readonly users: User[] = [];

  add(user: User): void {
    this.users.push(user);
  }
}
"#,
    ),
];

/// The synced fixture project, alive for the whole fuzz process.
pub struct Fixture {
    /// Releasing this would delete the project directory out from under the
    /// manager's database connection, so it is held for the process lifetime
    /// rather than for the duration of one call.
    _project: tempfile::TempDir,
    root: PathBuf,
    pub manager: SourceSyncManager,
}

impl Fixture {
    fn new() -> Self {
        let project = tempfile::Builder::new()
            .prefix("slugaudit-fuzz-")
            .tempdir()
            .expect("create fixture temp dir");
        let root = project.path().to_path_buf();

        // The activation directory is the marker `find_project_root` walks
        // ancestors for; without it every tool call fails before reaching the
        // code under test.
        std::fs::create_dir_all(root.join(".planning").join("slugaudit"))
            .expect("create activation dir");
        for (relative, content) in FILES {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("fixture file has a parent"))
                .expect("create fixture dir");
            std::fs::write(&path, content).expect("write fixture file");
        }

        let manager = SourceSyncManager::default();
        let path = path_str(&root);
        // Index once, before fuzzing starts, so the first iteration measures
        // the tool rather than the initial publish. A failed fixture sync
        // would make every fuzz input return an error and the run would pass
        // green while exercising nothing, so it aborts the fuzz process here.
        ensure_synced(&path, &NoopProgressSink, &manager)
            .expect("fixture sync must succeed or the fuzz run exercises nothing");

        Self {
            _project: project,
            root,
            manager,
        }
    }

    /// A path inside the active project, as the tools take it.
    pub fn path(&self) -> String {
        path_str(&self.root)
    }
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The process-wide fixture. `SourceSyncManager` is `Send + Sync` (the
/// server holds it across the blocking pool), so a plain `LazyLock` is
/// enough; that also keeps the targets valid if libFuzzer is ever run with
/// threads rather than `-jobs` processes.
pub static FIXTURE: LazyLock<Fixture> = LazyLock::new(Fixture::new);
