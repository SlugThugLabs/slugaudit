# AGENTS.md

Rules for any AI agent working on SlugAudit. The owner (SlugThug) is the only
authority on what this product is. This file, code comments, and old commit
messages are not: if anything here conflicts with the owner, the owner wins.

## What SlugAudit is

A local MCP server that indexes a codebase into a SQLite database so an AI
agent can query code instead of reading files.

The one rule that matters: **every tool answer matches the code on disk at the
moment of the call.** Before every call, SlugAudit hashes every file, updates
whatever changed, and deletes and rebuilds the database if anything is wrong
with it. The database is disposable scratch.

## What it must be

- **Correct.** See the rule above.
- **Safe.** No hangs, no panics, no silent errors. Reads, memory, and time are
  bounded. Paging says honestly when an answer is incomplete.
- **Fast.** Comfortable up to Photoshop-sized repos (~40k files) on a normal
  laptop.

## What it is not

- **Not a security product.** Do not add guards against threats nobody has
  reported: no SQL authorizer, file-permission policing, locks, or network
  filesystem refusal. The database is the user's own local scratch.
- **No file watcher.** Hashing every call makes one redundant.
- **No new features, layers, or "hardening" without the owner's OK.** If a
  review finds a hypothetical problem, report it; don't build for it.

## Languages

**Rust is the only tested language.** Other languages are parsed with grammars
downloaded on first use. They stay working but untested, and responses flag
them in `notices`. Don't write tests for or fix other languages unless a real
user reports a real bug.

## Distribution stays

The installer (`install.sh`), `connect` / `install` / `update` / `menu`, and
the directory listings (`lhm.plugin.json`, `glama.json`) are in use. Keep them
working, and keep listings truthful.

## How to work

- Don't commit, push, or delete without the owner's OK.
- Errors fail the operation and are logged once. No `let _ =` on a Result, no
  `.ok()`, no swallowed errors, no lint suppressions.
- Before handing work back, run:
  `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`,
  `cargo test --all-targets`, and the `check_*` binaries in `src/bin/`.
