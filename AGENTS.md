# AGENTS.md

Rules for any AI agent working on SlugAudit. The owner (SlugThug) decides
what the product is. If anything here, in code comments, or in commit
history conflicts with the owner, the owner wins.

## How the product works

- SlugAudit is a local MCP server (stdio). It indexes a project into a
  SQLite database. The model queries the index through the tools instead of
  reading files.
  <!-- Owner decision (2026-10-31): the purpose is to save the model context and tokens on mundane, repetitive file reads, so its context goes to the audit itself. -->
- On every tool call (except `health`), before answering: walk the project,
  BLAKE3-hash every file, update the rows of files that changed, add new
  files, remove deleted ones, and re-hash stored text against its stored
  hash. A broken or missing database is deleted and rebuilt.
  <!-- Owner decision (2026-10-31): the database only matters if it is exactly the same as the code base. Checking every hash on every call is simpler than trusting any shortcut. -->
- A project is on when the directory `<root>/.planning/slugaudit/` exists.
  `project_control on` creates it. `project_control off` deletes it. Nothing
  else creates it.
  <!-- Owner decision (2026-10-31): so a global install does not index every repo. A repo opts in; off removes everything because nothing there is needed anymore. -->
- State the model sets through the tools (findings, mechanics, profile)
  lasts one session: it is wiped when the SlugAudit process starts.
  <!-- Owner decision (2026-10-31): the owner restarts sessions to get fresh eyes when a model starts letting things slide. Carrying a past session's findings or settings forward would bring the old conclusions back. -->
- Defaults set by the human live in `~/.slugthug/slugaudit/config.json` and
  `SLUGAUDIT_*` environment variables. The tools never write them, and the
  model-facing instructions don't mention them.
  <!-- Owner decision (2026-10-31): the human can set base defaults. The model sets its own session settings through the tools and does not need to know the global file exists. -->
- Other languages' grammars are downloaded on first use.

## Product rules

- Every tool answer matches the code on disk at the moment of the call.
- No hangs, no panics, no silent errors. Reads, memory, and time are
  bounded. A page says when an answer is incomplete.
- Target: comfortable up to ~40k files on a normal laptop.
- No file watcher.
  <!-- Owner decision (2026-10-31): checking every file's hash on every call makes a watcher duplicate work. -->
- No guards against threats nobody reported: no SQL authorizer, file
  permission policing, locks, or network-filesystem refusal.
  <!-- Owner decision (2026-10-31): SlugAudit is not a security product and never was. The database is the user's own local scratch. Earlier agents kept building against threats nobody reported. -->
- No feature that ranks, scores, decides what matters, or gives the model a
  checklist. Built-in leads only count. The model chooses its own signals.
  <!-- Owner decision (2026-10-31): when the tool hands the model findings or a checklist, the model starts to treat running the tool as doing the audit. When the model sets up its own signals, the tool stays a helper. -->
- Rust is the only tested language. Other languages stay working, are
  flagged in `notices`, and get no new tests or fixes unless a user reports
  a bug.
  <!-- Owner decision (2026-10-31): the owner was tired of chasing hundreds of targets an LLM made up. Get Rust fully working first; other languages stay available, untested. -->
- Keep the installer (`install.sh`), `connect` / `install` / `update` /
  `menu`, and the listings (`lhm.plugin.json`, `glama.json`) working. A
  listing states only what the code does.
  <!-- Owner decision (2026-10-31): SlugAudit is listed on Glama and three other directories, and people install it from them. -->
- No new features, layers, or hardening without the owner's OK. A review
  that finds a hypothetical problem reports it and builds nothing.

## Working rules

- A line marked `Owner decision` records why the owner chose a rule. Don't
  change the rule on your own. If you think it should change, quote the
  decision to the owner and ask.
- Don't commit, push, or delete without the owner's OK.
- When something fails, check your own change first, then the test, then
  the environment.
- Existing code is not a reason to keep code. Before fixing something, check
  whether it should exist.
- No mocks, stubs, placeholder returns or fake data outside tests. Product
  code does the real work or fails with an error.
- Don't rewrite working code without a reason: a bug, a broken rule, or the
  owner's request. A function or file that mixes separate responsibilities is
  a broken rule; length alone is not. Never split code only to meet a line
  count.
  <!-- Owner decision (2026-11-02): over 200 lines per file or 50 per function is fine when the code does one job. The limits exist to stop god files, not to force splits. -->
- Never write a judgment about the code (for example, "module X is fine")
  into a file that outlives the session.
- Errors fail the operation and are logged once. No `let _ =` on a Result,
  no `.ok()`, no swallowed errors, no lint suppressions.
- Before handing work back, run:
  `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --all-targets`, and the `check_*` binaries in `src/bin/`.
