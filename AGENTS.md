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
  BLAKE3-hash every regular file on disk, compare against `files.content_hash`
  in SQLite, and update changed rows, add new files, and remove deleted ones.
  Never re-read or re-hash stored `files.content` out of SQLite. A broken or
  missing database is deleted and rebuilt.
  <!-- Owner decision (2026-10-31, clarified 2026-11-26): every file row holds its content hash so we hash disk files on every call and only touch rows whose hash differs. -->
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
- How the MCP server itself works (starting, the stdio transport, the
  session lifecycle, shutdown) is set by the MCP spec and handled by the
  `rmcp` SDK. That is not SlugAudit. SlugAudit is what runs on it: the
  index, the sync, and the tools.
  <!-- Owner decision (2026-11-03): that is how MCP servers are designed to work, and it is not us. -->


## Product rules

- Every tool answer matches the code on disk at the moment of the call.
- No hangs, no panics, no silent errors. Reads, memory, and time are
  bounded. A page says when an answer is incomplete.
- Target: comfortable up to ~40k files on a normal laptop.
- No file watcher.
  <!-- Owner decision (2026-10-31): checking every file's hash on every call makes a watcher duplicate work. -->
- No guards against threats nobody reported, and no protecting SlugAudit's
  scratch files against the model or local user: no SQL authorizer, workspace
  path jails, symlink/`NOFOLLOW` refusals, file-permission policing, locks
  beyond dead-session cleanup, or network-filesystem refusal.
  <!-- Owner decision (2026-10-31, clarified 2026-11-26): the host coding harness already owns the permission boundary, and the model already has access to the repo files. `project.db` and `results/` are disposable scratch holding only data from that repo. Bound accidental runaway calls (read-only `query`, time/step/byte caps); never build defenses against the model tampering with our own scratch DB or layers to police our own layers. -->
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

- Don't use SlugAudit's tools on this repo. Read and search the code
  directly.
  <!-- Owner decision (2026-11-03): you can't use a tool that isn't finished on itself while you build it, and doing so mixes up what SlugAudit does with the code that builds it. -->

- A line marked `Owner decision` records why the owner chose a rule. Don't
  change the rule on your own, and never add or rewrite an `Owner decision`
  comment unless recording the owner's explicit words with their OK. If you
  think a rule should change, quote the decision to the owner and ask.
- Don't commit, push, or delete without the owner's OK.
- When something fails, check your own change first, then the test, then
  the environment.
- Existing code is not a reason to keep code. Before fixing something, check
  whether it should exist.
- No mocks, stubs, placeholder returns or fake data outside tests. Product
  code does the real work or fails with an error.
- Never fabricate or edit benchmark baselines, coverage thresholds, or CI
  configs without running the tool that checks them (if the tool is not
  installed locally, stop and tell the owner). Never mask new wire fields out
  of golden test comparisons.
- Don't rewrite working code without a reason: a bug, a broken rule, or the
  owner's request. A function or file that mixes separate responsibilities is
  a broken rule; length alone is not. Never split code only to meet a line
  count. For any file at or above 200 code lines, run `split-judge`; if it
  says `KEEP`, record a `slugaudit-line-exception: approved-by=split-judge; reason=...`
  comment — there is no hard cap. Never stamp `approved-by=slugthuglabs` or
  `approved-by=human-user` on new or rewritten files.
  <!-- Owner decision (2026-11-02, updated 2026-11-26): over 200 lines per file or 50 per function is fine when the code does one job. No hard cap; run split-judge on files over 200 and trust its verdict. -->
- Never write a judgment about the code (for example, "module X is fine")
  into a file that outlives the session.
- Errors fail the operation and are logged once. No `let _ =` on a Result,
  no `.ok()`, no swallowed errors, no lint suppressions.
- Before handing work back, run:
  `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --all-targets`, and the gate binaries, from the repo
  root: `cargo run -q -p slugaudit-gates --bin <name>` for `check_source_limits`,
  `check_no_duplicates`, `check_producer_coverage` and `check_mcp_surface`
  (they live in `crates/gates/`, not in the product).
